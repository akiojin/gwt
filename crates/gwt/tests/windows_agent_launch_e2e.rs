//! SPEC-1921 Phase L1b: credential-free Windows installed-provider launch E2E.
//!
//! CI runs both providers through `GWT_WINDOWS_AGENT_PROVIDER`. The loopback
//! registry installs a fixture CLI before launch; every launch then probes and
//! dispatches the installed direct command without package-runner activity.
//!
//! Positive evidence is deliberately compositional: the 32-case matrix drives
//! the exact shared launch boundary, the source contract proves every reachable
//! app route converges on that boundary without a process bypass, and the public
//! WebSocket fixture drives a real Board route through gwt/gwtd, ConPTY, and an
//! authenticated SessionStart. A route label by itself is never treated as
//! proof that the public route executed.

#![cfg(windows)]

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use gwt_agent::{prepare_agent_launch, AgentId, AgentLaunchBuilder, Session, SessionMode};
use gwt_core::test_support::{
    summarize_pty_output_for_diagnostics, WindowsNpmRegistryFixture, WindowsRealGwtFixture,
};
use gwt_terminal::{
    pty::{PtyHandle, SpawnConfig},
    TerminalError,
};

const PROVIDER_ENV: &str = "GWT_WINDOWS_AGENT_PROVIDER";
const CREDENTIAL_FREE_ENV: &str = "GWT_WINDOWS_AGENT_E2E_CREDENTIAL_FREE";
const TEST_FIXTURE_INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
// Keep the saturated-runner marker budget and report observed latency.
const TEST_AGENT_READY_TIMEOUT: Duration = Duration::from_secs(180);
// The direct provider process must exit after its readiness marker.
const TEST_PROVIDER_EXIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteMode {
    Fresh,
    Resume,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteCase {
    id: &'static str,
    mode: RouteMode,
}

const ROUTE_CASES: [RouteCase; 32] = [
    route("wizard.branch.configure", RouteMode::Fresh),
    route("wizard.work.configure", RouteMode::Fresh),
    route("wizard.intake.configure", RouteMode::Fresh),
    route("wizard.agent-kanban.configure", RouteMode::Fresh),
    route("wizard.knowledge.issue.configure", RouteMode::Fresh),
    route("wizard.knowledge.spec-selected.configure", RouteMode::Fresh),
    route("wizard.start-method.last-settings", RouteMode::Fresh),
    route(
        "wizard.start-method.continue-last.exact",
        RouteMode::Continue,
    ),
    route(
        "wizard.start-method.continue-last.latest",
        RouteMode::Continue,
    ),
    route("wizard.start-method.native-picker", RouteMode::Resume),
    route("monitor.manual.launch-now", RouteMode::Fresh),
    route("monitor.manual.quick-register-launch", RouteMode::Fresh),
    route("monitor.autonomous.implementation-fresh", RouteMode::Fresh),
    route("monitor.autonomous.review-fresh", RouteMode::Fresh),
    route(
        "monitor.autonomous.existing-resume.persisted",
        RouteMode::Resume,
    ),
    route("workspace-agent-resume.linked-exact", RouteMode::Resume),
    route("workspace-agent-resume.linked-handoff", RouteMode::Resume),
    route("workspace-agent-resume.legacy-exact", RouteMode::Resume),
    route("workspace-agent-resume.legacy-picker", RouteMode::Resume),
    route("branch-latest-resume.persisted-resume", RouteMode::Resume),
    route("board-origin-resume.persisted-resume", RouteMode::Resume),
    route("continue-work.continued-conversation", RouteMode::Continue),
    route("continue-work.started-with-handoff", RouteMode::Fresh),
    route("restore.startup-auto-resume", RouteMode::Resume),
    route("restore.open-project.exact", RouteMode::Resume),
    route("restore.open-project.fresh-fallback", RouteMode::Fresh),
    route("restore.reopen-recent.exact", RouteMode::Resume),
    route("restore.reopen-recent.fresh-fallback", RouteMode::Fresh),
    route("restore.restart-stopped.exact", RouteMode::Resume),
    route("restore.restart-stopped.fresh-fallback", RouteMode::Fresh),
    route("restore.restart-error.exact", RouteMode::Resume),
    route("restore.restart-error.fresh-fallback", RouteMode::Fresh),
];

const fn route(id: &'static str, mode: RouteMode) -> RouteCase {
    RouteCase { id, mode }
}

impl Provider {
    fn from_env() -> Self {
        match required_env(PROVIDER_ENV).as_str() {
            "codex" => Self::Codex,
            "claude" => Self::Claude,
            value => panic!("{PROVIDER_ENV} must be codex or claude, got {value:?}"),
        }
    }

    fn agent_id(self) -> AgentId {
        match self {
            Self::Codex => AgentId::Codex,
            Self::Claude => AgentId::ClaudeCode,
        }
    }

    fn package(self) -> &'static str {
        match self {
            Self::Codex => "@openai/codex",
            Self::Claude => "@anthropic-ai/claude-code",
        }
    }
}

fn required_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set for an explicit CI shard"))
}

#[test]
#[ignore = "installs a fixture CLI from a loopback registry, then runs direct Windows launches"]
fn windows_official_provider_launch_uses_installed_direct_runner() {
    init_test_tracing();
    assert_eq!(
        required_env(CREDENTIAL_FREE_ENV),
        "1",
        "the Windows launch E2E must run in credential-free mode"
    );
    let provider = Provider::from_env();
    let temp = tempfile::tempdir().expect("Phase 75 E2E tempdir");
    let fixture = WindowsNpmRegistryFixture::create(temp.path())
        .expect("create loopback npm registry fixture");
    let worktree = temp.path().join("作業ツリー with spaces");
    std::fs::create_dir_all(&worktree).expect("create E2E worktree");
    let git_init = gwt_core::process::hidden_command("git")
        .args(["init", "--quiet"])
        .current_dir(&worktree)
        .output()
        .expect("initialize E2E Git worktree");
    assert!(
        git_init.status.success(),
        "initialize E2E Git worktree: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );
    let sessions_dir = fixture.profile.join(".gwt").join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create isolated Session directory");
    let _hook_bin =
        gwt_core::test_support::ScopedEnvVar::set("GWT_HOOK_BIN", env!("CARGO_BIN_EXE_gwtd"));
    let installed_bin = install_fixture_provider(&fixture, provider);
    let launch_env = launch_env_with_real_gwtd(&fixture, &installed_bin);
    let registry_requests_after_install = fixture.requests().len();
    assert_canonical_route_manifest();
    let public_route_only = std::env::var_os("GWT_WINDOWS_AGENT_E2E_PUBLIC_ROUTE_ONLY")
        .is_some_and(|value| value == "1");

    let mut executed_route_count = 0;
    if !public_route_only {
        for route in ROUTE_CASES {
            run_shared_launch_boundary_case(
                route,
                provider,
                &fixture,
                &launch_env,
                &worktree,
                &sessions_dir,
                temp.path(),
            );
            executed_route_count += 1;
        }
        assert_eq!(
            executed_route_count,
            ROUTE_CASES.len(),
            "every advertised route must execute the shared launch boundary"
        );
    }

    assert_public_ws_agent_route_boundary(temp.path(), provider, &fixture, &launch_env);

    assert_eq!(
        fixture.requests().len(),
        registry_requests_after_install,
        "installed launches must not access the registry"
    );
    let requests = fixture.requests();
    assert!(!requests.is_empty(), "npm must use the loopback registry");
    assert!(
        requests.iter().all(|request| {
            !request.headers.contains_key("authorization")
                && !request.headers.contains_key("proxy-authorization")
        }),
        "loopback npm requests must not carry ambient credentials: {requests:?}"
    );
    assert!(
        requests.iter().any(|request| {
            let path = request.path.to_ascii_lowercase();
            path.contains("%2f") || path.contains(provider.package())
        }),
        "registry log must contain official-package packument access: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.path.ends_with(".tgz")),
        "registry log must contain a package tarball access: {requests:?}"
    );
    assert!(
        !fixture.bunx_marker.exists(),
        "the bunx tripwire fired during the route matrix"
    );
}

fn assert_public_ws_agent_route_boundary(
    temp_root: &Path,
    provider: Provider,
    npm_fixture: &WindowsNpmRegistryFixture,
    launch_env: &std::collections::HashMap<String, String>,
) {
    let workspace = temp_root.join("public ws 作業ツリー");
    let home = temp_root.join("public ws browser profile");
    std::fs::create_dir_all(&workspace).expect("create public WS workspace");
    let git_init = gwt_core::process::hidden_command("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace)
        .output()
        .expect("initialize public WS Git workspace");
    assert!(
        git_init.status.success(),
        "initialize public WS Git workspace: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );
    let sessions_dir = home.join(".gwt").join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create public WS sessions directory");
    std::fs::copy(&npm_fixture.npmrc, home.join(".npmrc"))
        .expect("install isolated loopback npmrc for the real gwt route");
    let provider_session_id = format!("phase75-public-{}-installed", provider.agent_id().command());
    let mut source = Session::new(&workspace, "master", provider.agent_id());
    source.agent_session_id = Some(format!("source-{provider_session_id}"));
    source.tool_version = Some(npm_fixture.exact_version.clone());
    source
        .save(&sessions_dir)
        .expect("persist public WS Board origin Session");
    let capture = temp_root.join("public ws agent receipt.json");
    let mut public_env = launch_env.clone();
    public_env.insert(
        "GWT_PHASE75_CAPTURE".to_string(),
        capture.to_string_lossy().into_owned(),
    );
    public_env.insert(
        "GWT_PHASE75_PROVIDER_SESSION_ID".to_string(),
        provider_session_id.clone(),
    );
    public_env.insert("GWT_PHASE75_HOLD_OPEN_MS".to_string(), "30000".to_string());
    let fixture = WindowsRealGwtFixture::start(
        &temp_root.join("real gwt fixture"),
        Path::new(env!("CARGO_BIN_EXE_gwt")),
        &home,
        &workspace,
        &public_env,
    )
    .expect("start isolated real gwt.exe");
    let expectation = PublicRouteExpectation {
        ws_url: fixture.public_ws_url(),
        workspace: fixture.workspace.to_string_lossy().into_owned(),
        origin_session_id: source.id,
        capture,
        provider,
        observed_version: npm_fixture.exact_version.clone(),
        provider_session_id,
        sessions_dir,
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Phase 75 Tokio runtime")
        .block_on(assert_public_ws_conpty_boundary_async(expectation));
    if let Err(error) = result {
        panic!(
            "real gwt public /ws -> ConPTY boundary failed: {error}\ngwt stderr:\n{}",
            fixture.stderr()
        );
    }
}

/// Issue #4644: since #4537 the bootstrap `/ws` connection is Hub-only. It
/// answers `frontend_ready` with the lightweight `hub_state` catalog (the
/// browser's `receiveHubState`); project `workspace_state` is sent only on a
/// connection bound to a project key.
#[derive(Debug, PartialEq, Eq)]
enum HubBootstrapReply {
    Synced,
    Pending,
}

fn classify_hub_bootstrap_reply(kind: &str) -> Result<HubBootstrapReply, String> {
    match kind {
        "hub_state" => Ok(HubBootstrapReply::Synced),
        "workspace_state" => Err(
            "Hub bootstrap connection received project-scoped workspace_state; \
             the Hub contract (#4537) syncs with hub_state only"
                .to_string(),
        ),
        _ => Ok(HubBootstrapReply::Pending),
    }
}

fn hub_project_keys(hub_state: &serde_json::Value) -> Vec<String> {
    hub_state["hub"]["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|project| project["project_key"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn hub_bootstrap_reply_names_the_scope_violation_instead_of_timing_out() {
    assert_eq!(
        classify_hub_bootstrap_reply("hub_state"),
        Ok(HubBootstrapReply::Synced)
    );
    for pending in ["runtime_health", "provider_usage", "update_state"] {
        assert_eq!(
            classify_hub_bootstrap_reply(pending),
            Ok(HubBootstrapReply::Pending)
        );
    }
    let error = classify_hub_bootstrap_reply("workspace_state").unwrap_err();
    assert!(error.contains("hub_state"), "{error}");
    assert_eq!(
        hub_project_keys(&serde_json::json!({
            "kind": "hub_state",
            "hub": {"projects": [{"project_key": "0123456789abcdef"}]},
        })),
        vec!["0123456789abcdef".to_string()]
    );
}

struct PublicRouteExpectation {
    ws_url: String,
    workspace: String,
    origin_session_id: String,
    capture: PathBuf,
    provider: Provider,
    observed_version: String,
    provider_session_id: String,
    sessions_dir: PathBuf,
}

async fn assert_public_ws_conpty_boundary_async(
    expectation: PublicRouteExpectation,
) -> Result<(), String> {
    let PublicRouteExpectation {
        ws_url,
        workspace,
        origin_session_id,
        capture,
        provider,
        observed_version,
        provider_session_id,
        sessions_dir,
    } = expectation;
    let (mut socket, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|error| format!("connect {ws_url}: {error}"))?;
    // The URL handoff is written immediately before tao enters `run()`. A
    // browser can therefore connect during that narrow startup seam. Re-send
    // the idempotent readiness handshake until the event loop proves it is
    // consuming frontend events instead of racing one pre-run send.
    let handshake_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut observed_kinds = Vec::new();
    let mut known_project_keys = None;
    while known_project_keys.is_none() && tokio::time::Instant::now() < handshake_deadline {
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"kind":"frontend_ready"}"#.into(),
            ))
            .await
            .map_err(|error| format!("send frontend_ready: {error}"))?;
        let attempt_deadline = std::cmp::min(
            tokio::time::Instant::now() + Duration::from_millis(500),
            handshake_deadline,
        );
        while tokio::time::Instant::now() < attempt_deadline {
            let remaining = attempt_deadline.saturating_duration_since(tokio::time::Instant::now());
            let Ok(message) = tokio::time::timeout(remaining, socket.next()).await else {
                break;
            };
            let message = message
                .ok_or_else(|| "public websocket closed during frontend handshake".to_string())?
                .map_err(|error| format!("read frontend handshake: {error}"))?;
            let Some(payload) = message.to_text().ok() else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                continue;
            };
            if let Some(kind) = value["kind"].as_str() {
                observed_kinds.push(kind.to_string());
                if classify_hub_bootstrap_reply(kind)? == HubBootstrapReply::Synced {
                    known_project_keys = Some(hub_project_keys(&value));
                    break;
                }
            }
        }
    }
    let Some(known_project_keys) = known_project_keys else {
        return Err(format!(
            "Hub bootstrap connection never answered frontend_ready with hub_state; \
             observed={observed_kinds:?}"
        ));
    };
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "kind": "reopen_recent_project",
                "path": workspace,
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("send reopen_recent_project: {error}"))?;
    let project_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let canonical_workspace = std::fs::canonicalize(&workspace)
        .map_err(|error| format!("canonicalize public workspace: {error}"))?;
    let mut project_key = None;
    let mut project_event_kinds = Vec::new();
    while project_key.is_none() && tokio::time::Instant::now() < project_deadline {
        let remaining = project_deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| {
                format!(
                    "timed out waiting for the opened project in hub_state; observed={project_event_kinds:?}"
                )
            })?
            .ok_or_else(|| "public websocket closed before project open".to_string())?
            .map_err(|error| format!("read project hub_state: {error}"))?;
        let Some(payload) = message.to_text().ok() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        let Some(kind) = value["kind"].as_str() else {
            continue;
        };
        project_event_kinds.push(kind.to_string());
        if kind == "project_open_error" {
            return Err(format!("reopen_recent_project failed: {value}"));
        }
        if classify_hub_bootstrap_reply(kind)? == HubBootstrapReply::Synced {
            project_key = hub_project_keys(&value)
                .into_iter()
                .find(|key| !known_project_keys.contains(key));
        }
    }
    let project_key =
        project_key.ok_or_else(|| "project tab key was not materialized".to_string())?;
    // Match the browser: the bootstrap connection is Hub-only; project
    // mutations require a new connection bound to the materialized key.
    socket
        .close(None)
        .await
        .map_err(|error| format!("close Hub websocket: {error}"))?;
    let mut project_ws_url = reqwest::Url::parse(&ws_url)
        .map_err(|error| format!("parse public websocket URL: {error}"))?;
    project_ws_url
        .query_pairs_mut()
        .append_pair("repo_hash", &project_key);
    let (mut socket, _) = tokio_tungstenite::connect_async(project_ws_url.as_str())
        .await
        .map_err(|error| format!("connect project websocket: {error}"))?;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"kind":"frontend_ready"}"#.into(),
        ))
        .await
        .map_err(|error| format!("send project frontend_ready: {error}"))?;

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"kind":"create_window","preset":"board","bounds":{"x":0,"y":0,"width":1000,"height":700}}"#.into(),
        ))
        .await
        .map_err(|error| format!("send create Board window: {error}"))?;
    let board_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut board_id = None;
    while board_id.is_none() && tokio::time::Instant::now() < board_deadline {
        let remaining = board_deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| "timed out waiting for Board workspace_state".to_string())?
            .ok_or_else(|| "public websocket closed before Board creation".to_string())?
            .map_err(|error| format!("read Board workspace_state: {error}"))?;
        let Some(payload) = message.to_text().ok() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if value["kind"] != "workspace_state" {
            continue;
        }
        let tab = &value["workspace"]["tabs"][0];
        let tab_root = tab["project_root"]
            .as_str()
            .and_then(|path| std::fs::canonicalize(Path::new(path)).ok());
        if tab["project_key"].as_str() != Some(project_key.as_str())
            || tab_root.as_deref() != Some(canonical_workspace.as_path())
        {
            return Err(format!(
                "project connection {project_key} served another project: {}",
                value["workspace"]["tabs"]
            ));
        }
        board_id = value["workspace"]["tabs"]
            .as_array()
            .and_then(|tabs| {
                tabs.iter()
                    .filter_map(|tab| tab["workspace"]["windows"].as_array())
                    .flat_map(|windows| windows.iter())
                    .find(|window| window["preset"] == "board")
                    .and_then(|window| window["id"].as_str())
            })
            .map(str::to_string);
    }
    let board_id = board_id.ok_or_else(|| "Board window was not materialized".to_string())?;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "kind": "open_board_origin_agent",
                "id": board_id.clone(),
                "origin_session_id": origin_session_id,
                "bounds": null,
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("send open_board_origin_agent: {error}"))?;

    let agent_deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let mut route_events = Vec::new();
    let mut route_diagnostics = Vec::new();
    let mut agent_dsr_replied = false;
    while !capture.is_file() && tokio::time::Instant::now() < agent_deadline {
        let attempt_deadline = std::cmp::min(
            tokio::time::Instant::now() + Duration::from_millis(250),
            agent_deadline,
        );
        let remaining = attempt_deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, socket.next()).await {
            Ok(Some(Ok(message))) => {
                if let Ok(payload) = message.to_text() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                        if let Some(kind) = value["kind"].as_str() {
                            route_events.push(kind.to_string());
                            if matches!(
                                kind,
                                "launch_progress" | "terminal_status" | "window_state"
                            ) {
                                route_diagnostics.push(value.to_string());
                            } else if matches!(kind, "terminal_output" | "terminal_snapshot") {
                                if let Some(encoded) = value["data_base64"].as_str() {
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(encoded)
                                    {
                                        if !agent_dsr_replied
                                            && bytes.windows(4).any(|window| window == b"\x1b[6n")
                                        {
                                            let terminal_id =
                                                value["id"].as_str().ok_or_else(|| {
                                                    "agent terminal output omitted its window id"
                                                        .to_string()
                                                })?;
                                            socket
                                                .send(
                                                    tokio_tungstenite::tungstenite::Message::Text(
                                                        serde_json::json!({
                                                            "kind": "terminal_input",
                                                            "id": terminal_id,
                                                            "data": "\u{1b}[1;1R",
                                                        })
                                                        .to_string()
                                                        .into(),
                                                    ),
                                                )
                                                .await
                                                .map_err(|error| {
                                                    format!(
                                                        "send agent terminal DSR response: {error}"
                                                    )
                                                })?;
                                            agent_dsr_replied = true;
                                        }
                                        route_diagnostics
                                            .push(String::from_utf8_lossy(&bytes).into_owned());
                                    }
                                }
                            }
                        }
                        if value["kind"] == "launch_error" || value["kind"] == "board_error" {
                            return Err(format!(
                                "public Board route failed before authenticated SessionStart: {value}"
                            ));
                        }
                        if value["kind"] == "terminal_status" && value["status"] == "error" {
                            return Err(format!(
                                "public Board route entered Error before authenticated SessionStart: {value}; diagnostics={route_diagnostics:?}"
                            ));
                        }
                    }
                }
            }
            Ok(Some(Err(error))) => {
                return Err(format!("read public Board route event: {error}"));
            }
            Ok(None) => {
                return Err("public websocket closed during Board route launch".to_string());
            }
            Err(_) => {}
        }
    }
    if !capture.is_file() {
        return Err(format!(
            "timed out waiting for public Board route provider receipt; observed={route_events:?}; diagnostics={route_diagnostics:?}"
        ));
    }
    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&capture)
            .map_err(|error| format!("read public Board route receipt: {error}"))?,
    )
    .map_err(|error| format!("parse public Board route receipt: {error}"))?;
    if receipt["package"] != provider.package()
        || receipt["version"] != observed_version
        || receipt["provider_session_id"] != provider_session_id
        || receipt["hook_status"].as_i64() != Some(0)
        || receipt["hook_forward_token_present"].as_bool() != Some(true)
        || receipt["tty"].as_bool() != Some(true)
    {
        return Err(format!(
            "public Board route receipt did not prove installed authenticated ConPTY launch: {receipt}"
        ));
    }
    let launched_session_id = receipt["gwt_session_id"]
        .as_str()
        .ok_or_else(|| format!("public Board route receipt omitted gwt Session id: {receipt}"))?;
    let persisted = Session::load(&sessions_dir.join(format!("{launched_session_id}.toml")))
        .map_err(|error| format!("load public Board route Session: {error}"))?;
    if persisted.agent_session_id.as_deref() != Some(provider_session_id.as_str())
        || persisted.tool_version.as_deref() != Some(observed_version.as_str())
        || persisted.tool_runtime_provenance.is_some()
        || receipt["npm_exec_identity"] != serde_json::Value::Null
    {
        return Err(format!(
            "public Board route Session did not commit authenticated installed identity: {persisted:?}"
        ));
    }
    let capture_before_focus = std::fs::read(&capture)
        .map_err(|error| format!("snapshot public Board route receipt: {error}"))?;
    let session_count_before_focus = std::fs::read_dir(&sessions_dir)
        .map_err(|error| format!("list Sessions before live focus: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
        })
        .count();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "kind": "open_board_origin_agent",
                "id": board_id,
                "origin_session_id": launched_session_id,
                "bounds": null,
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| format!("send live Board origin focus: {error}"))?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    let session_count_after_focus = std::fs::read_dir(&sessions_dir)
        .map_err(|error| format!("list Sessions after live focus: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
        })
        .count();
    if session_count_after_focus != session_count_before_focus
        || std::fs::read(&capture)
            .map_err(|error| format!("read receipt after live focus: {error}"))?
            != capture_before_focus
    {
        return Err(
            "live Board origin focus unexpectedly spawned or resolved another provider".to_string(),
        );
    }

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"kind":"create_window","preset":"shell","bounds":{"x":0,"y":0,"width":1000,"height":700}}"#.into(),
        ))
        .await
        .map_err(|error| format!("send create_window: {error}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut shell_id = None;
    while shell_id.is_none() && tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| "timed out waiting for shell workspace_state".to_string())?
            .ok_or_else(|| "public websocket closed before shell creation".to_string())?
            .map_err(|error| format!("read workspace_state: {error}"))?;
        let Some(payload) = message.to_text().ok() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if value["kind"] != "workspace_state" {
            continue;
        }
        shell_id = value["workspace"]["tabs"]
            .as_array()
            .and_then(|tabs| {
                tabs.iter()
                    .filter_map(|tab| tab["workspace"]["windows"].as_array())
                    .flat_map(|windows| windows.iter())
                    .find(|window| window["preset"] == "shell")
                    .and_then(|window| window["id"].as_str())
            })
            .map(str::to_string);
    }
    let shell_id = shell_id.ok_or_else(|| "shell window was not materialized".to_string())?;
    let marker = "PHASE75-PUBLIC-WS-CONPTY";
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut decoded_output = String::new();
    let mut dsr_replied = false;
    let mut command_sent = false;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(remaining, socket.next())
            .await
            .map_err(|_| format!("timed out waiting for marker; output={decoded_output:?}"))?
            .ok_or_else(|| "public websocket closed before terminal output".to_string())?
            .map_err(|error| format!("read terminal output: {error}"))?;
        let Some(payload) = message.to_text().ok() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        if value["kind"] == "terminal_status"
            && value["id"].as_str() == Some(shell_id.as_str())
            && value["status"] == "running"
            && !command_sent
        {
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "kind": "terminal_input",
                        "id": shell_id,
                        "data": format!("echo {marker}\r"),
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .map_err(|error| format!("send terminal_input after Running: {error}"))?;
            command_sent = true;
            continue;
        }
        if !matches!(
            value["kind"].as_str(),
            Some("terminal_output" | "terminal_snapshot")
        ) || value["id"].as_str() != Some(shell_id.as_str())
        {
            continue;
        }
        let Some(encoded) = value["data_base64"].as_str() else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("decode terminal output: {error}"))?;
        decoded_output.push_str(&String::from_utf8_lossy(&bytes));
        if !dsr_replied && decoded_output.contains("\u{1b}[6n") {
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "kind": "terminal_input",
                        "id": shell_id,
                        "data": "\u{1b}[1;1R",
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .map_err(|error| format!("send terminal DSR response: {error}"))?;
            dsr_replied = true;
        }
        if !command_sent {
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::json!({
                        "kind": "terminal_input",
                        "id": shell_id,
                        "data": format!("echo {marker}\r"),
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .map_err(|error| format!("send terminal_input after PTY output: {error}"))?;
            command_sent = true;
        }
        if decoded_output.contains(marker) {
            return Ok(());
        }
    }
    Err(format!(
        "terminal marker was not observed; output={decoded_output:?}"
    ))
}

#[allow(clippy::too_many_arguments)]
fn run_shared_launch_boundary_case(
    route: RouteCase,
    provider: Provider,
    fixture: &WindowsNpmRegistryFixture,
    launch_env: &std::collections::HashMap<String, String>,
    worktree: &Path,
    sessions_dir: &Path,
    temp_root: &Path,
) {
    let capture = route_capture_path(temp_root, route.id);
    std::fs::create_dir_all(capture.parent().expect("receipt directory"))
        .expect("create route receipt directory");
    let prompt = format!("route {}: $gwt-execute #3152 — café 日本語", route.id);
    let provider_session_id = format!("phase75-{}", route.id.replace('.', "-"));
    let mut builder = AgentLaunchBuilder::new(provider.agent_id())
        .working_dir(worktree)
        .extra_arg(&prompt)
        .env("GWT_PHASE75_CAPTURE", capture.to_string_lossy())
        .env("GWT_PHASE75_PROVIDER_SESSION_ID", &provider_session_id);
    builder = match route.mode {
        RouteMode::Fresh => builder,
        RouteMode::Resume => builder
            .session_mode(SessionMode::Resume)
            .resume_session_id(format!("resume-{}", route.id)),
        RouteMode::Continue => builder.session_mode(SessionMode::Continue),
    };
    for (key, value) in launch_env {
        builder = builder.env(key, value);
    }
    let mut config = builder.build();
    config.remove_env.extend(credential_env_removals());

    let prepared = prepare_agent_launch(worktree, sessions_dir, config, None, |path| {
        gwt::refresh_managed_gwt_assets_for_agent(path, &provider.agent_id())
            .map_err(|error| error.to_string())
    })
    .unwrap_or_else(|error| panic!("{} installed launch preparation failed: {error}", route.id));
    let expected_command = format!("{}.cmd", provider.agent_id().command());
    assert!(runner_file_name_is(
        &prepared.process_launch.command,
        &expected_command
    ));
    assert_eq!(
        prepared.session.tool_version.as_deref(),
        Some(fixture.exact_version.as_str())
    );
    assert!(prepared.session.tool_runtime_provenance.is_none());
    assert_eq!(
        prepared
            .process_launch
            .args
            .iter()
            .filter(|arg| arg.as_str() == prompt)
            .count(),
        1,
        "{} must preserve the prompt as one argv element",
        route.id
    );

    let pty = PtyHandle::spawn(SpawnConfig {
        command: prepared.process_launch.command.clone(),
        args: prepared.process_launch.args.clone(),
        cols: 120,
        rows: 40,
        env: prepared.process_launch.env.clone(),
        remove_env: prepared.process_launch.remove_env.clone(),
        cwd: prepared.process_launch.cwd.clone(),
    })
    .unwrap_or_else(|error| panic!("{} ConPTY spawn failed: {error}", route.id));
    // Windows ConPTY shells issue a cursor-position query before dispatching
    // the child command. xterm.js answers this in production; the headless
    // harness supplies the same terminal response explicitly.
    pty.write_input(b"\x1b[1;1R")
        .unwrap_or_else(|error| panic!("{} ConPTY DSR response failed: {error}", route.id));
    let output = read_pty_until_marker(pty, "phase75-agent-ready", TEST_AGENT_READY_TIMEOUT)
        .unwrap_or_else(|error| panic!("{} ConPTY output failed: {error}", route.id));
    assert!(
        output.contains("phase75-agent-ready"),
        "{} did not reach fake provider readiness through ConPTY: {output}",
        route.id
    );

    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&capture).expect("fake provider package must write a receipt"),
    )
    .expect("provider receipt JSON");
    let argv = receipt["argv"].as_array().expect("provider argv array");
    assert_eq!(
        argv.last().and_then(serde_json::Value::as_str),
        Some(prompt.as_str()),
        "{} lost the final prompt boundary",
        route.id
    );
    assert_eq!(receipt["package"], provider.package());
    assert_eq!(receipt["version"], fixture.exact_version);
    assert_eq!(receipt["hook_status"].as_i64(), Some(0));
    assert_eq!(
        receipt["codex_thread_id"],
        serde_json::Value::Null,
        "credential-free child must not inherit the parent Codex identity: {receipt}"
    );
    assert_eq!(receipt["provider_session_id"], provider_session_id);
    assert_eq!(
        receipt["hook_forward_token_present"].as_bool(),
        Some(false),
        "credential-free route must not inherit the parent gwt capability"
    );
    assert_eq!(receipt["tty"].as_bool(), Some(true));
    assert_eq!(
        receipt["gwt_bin_name"].as_str(),
        Some("gwtd.exe"),
        "generated hook must execute the real sibling gwtd binary"
    );
    assert_eq!(
        dunce::canonicalize(
            receipt["gwt_bin_path"]
                .as_str()
                .expect("provider receipt must capture GWT_BIN_PATH")
        )
        .expect("canonicalize provider GWT_BIN_PATH"),
        dunce::canonicalize(env!("CARGO_BIN_EXE_gwtd")).expect("canonicalize checkout gwtd"),
        "generated hook must execute this checkout's gwtd binary"
    );
    assert_eq!(receipt["npm_exec_identity"], serde_json::Value::Null);
    assert_eq!(
        receipt["project_root"].as_str(),
        Some(worktree.to_string_lossy().as_ref())
    );

    let persisted = Session::load(&sessions_dir.join(format!("{}.toml", prepared.session.id)))
        .expect("load Session after generated SessionStart hook");
    assert_eq!(
        persisted.agent_session_id.as_deref(),
        Some(provider_session_id.as_str()),
        "{} must persist authenticated provider SessionStart identity; receipt={receipt}",
        route.id,
    );
    assert!(
        persisted
            .session_history
            .iter()
            .any(|entry| entry.agent_session_id == provider_session_id),
        "{} must append provider identity to Session history",
        route.id
    );
    assert!(persisted.tool_runtime_provenance.is_none());
    assert_eq!(
        persisted.tool_version.as_deref(),
        Some(fixture.exact_version.as_str())
    );
    eprintln!("phase75 installed launch boundary PASS: {}", route.id);
}

fn read_pty_until_marker(
    pty: PtyHandle,
    marker: &str,
    timeout: Duration,
) -> Result<String, TerminalError> {
    let mut reader = pty.reader()?;
    let (sender, receiver) = mpsc::channel();
    let reader_thread = std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(Ok(Vec::new()));
                    break;
                }
                Ok(count) => {
                    if sender.send(Ok(buffer[..count].to_vec())).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                    break;
                }
            }
        }
    });
    let wait_started = Instant::now();
    let marker_deadline = wait_started + timeout;
    let mut exit_deadline = None;
    let mut bytes = Vec::new();
    let result = loop {
        if exit_deadline.is_some() {
            match pty.try_wait() {
                Ok(Some(_)) => break Ok(String::from_utf8_lossy(&bytes).into_owned()),
                Ok(None) => {}
                Err(error) => break Err(error),
            }
        }
        let active_deadline = exit_deadline.unwrap_or(marker_deadline);
        let Some(remaining) = active_deadline.checked_duration_since(Instant::now()) else {
            break Err(TerminalError::PtyIoError {
                details: if exit_deadline.is_some() {
                    format!(
                        "PTY marker {marker:?} was observed but provider did not exit cleanly; waited={:?} budget={TEST_PROVIDER_EXIT_TIMEOUT:?}; {}",
                        wait_started.elapsed(),
                        summarize_pty_output_for_diagnostics(&bytes)
                    )
                } else {
                    format!(
                        "timed out waiting for PTY marker {marker:?}; waited={:?} budget={timeout:?}; {}",
                        wait_started.elapsed(),
                        summarize_pty_output_for_diagnostics(&bytes)
                    )
                },
            });
        };
        let receive_for = if exit_deadline.is_some() {
            remaining.min(Duration::from_millis(100))
        } else {
            remaining
        };
        match receiver.recv_timeout(receive_for) {
            Ok(Ok(chunk)) if chunk.is_empty() => {
                if exit_deadline.is_some() {
                    continue;
                }
                break Err(TerminalError::PtyIoError {
                    details: format!(
                        "PTY reached EOF before marker {marker:?}; waited={:?} budget={timeout:?}; {}",
                        wait_started.elapsed(),
                        summarize_pty_output_for_diagnostics(&bytes)
                    ),
                });
            }
            Ok(Ok(chunk)) => {
                bytes.extend_from_slice(&chunk);
                if exit_deadline.is_none() && String::from_utf8_lossy(&bytes).contains(marker) {
                    eprintln!(
                        "phase75 marker timing: marker={marker:?} elapsed_ms={} budget_ms={}",
                        wait_started.elapsed().as_millis(),
                        timeout.as_millis()
                    );
                    // Wait for the installed CLI to exit before the next route starts.
                    exit_deadline = Some(Instant::now() + TEST_PROVIDER_EXIT_TIMEOUT);
                }
            }
            Ok(Err(error)) => {
                break Err(TerminalError::PtyIoError { details: error });
            }
            Err(mpsc::RecvTimeoutError::Timeout) if exit_deadline.is_some() => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                break Err(TerminalError::PtyIoError {
                    details: format!(
                        "timed out waiting for PTY marker {marker:?}; waited={:?} budget={timeout:?}; {}",
                        wait_started.elapsed(),
                        summarize_pty_output_for_diagnostics(&bytes)
                    ),
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) if exit_deadline.is_some() => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(TerminalError::PtyIoError {
                    details: "PTY reader disconnected before marker".to_string(),
                });
            }
        }
    };
    if result.is_err() {
        let _ = pty.kill();
    }
    drop(pty);
    let _ = reader_thread.join();
    result
}

fn launch_env_with_real_gwtd(
    fixture: &WindowsNpmRegistryFixture,
    installed_bin: &Path,
) -> std::collections::HashMap<String, String> {
    let mut env = fixture.launch_env();
    let gwtd_dir = Path::new(env!("CARGO_BIN_EXE_gwtd"))
        .parent()
        .expect("gwtd binary parent");
    let current = env.get("PATH").map(String::as_str).unwrap_or_default();
    let mut paths = vec![installed_bin.to_path_buf(), gwtd_dir.to_path_buf()];
    paths.extend(std::env::split_paths(current));
    env.insert(
        "PATH".to_string(),
        std::env::join_paths(paths)
            .expect("E2E PATH")
            .to_string_lossy()
            .into_owned(),
    );
    // `prepare_agent_launch` normally discovers the sibling `gwtd` from the
    // running `gwt.exe`. This integration test runs inside a Cargo test
    // executable, so pin the equivalent production value explicitly instead
    // of allowing an unrelated globally installed `gwtd` to satisfy lookup.
    env.insert(
        "GWT_BIN_PATH".to_string(),
        env!("CARGO_BIN_EXE_gwtd").to_string(),
    );
    env
}

fn init_test_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_target(true)
        .with_test_writer()
        .try_init();
}

fn credential_env_removals() -> Vec<String> {
    [
        "CODEX_THREAD_ID",
        "NODE_AUTH_TOKEN",
        "NPM_TOKEN",
        "NPM_AUTH_TOKEN",
        "NPM_CONFIG__AUTH",
        "NPM_CONFIG__AUTHTOKEN",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn install_fixture_provider(fixture: &WindowsNpmRegistryFixture, provider: Provider) -> PathBuf {
    use gwt_core::process_console::{
        spawn_logged_blocking_with_deadline, ProcessConsoleHub, ProcessKind, SpawnOptions,
    };
    let prefix = fixture.profile.join("installed provider");
    std::fs::create_dir_all(&prefix).expect("create installed provider prefix");
    let args = vec![
        "install".to_string(),
        "--prefix".to_string(),
        prefix.to_string_lossy().into_owned(),
        "--ignore-scripts".to_string(),
        "--no-audit".to_string(),
        "--no-fund".to_string(),
        "--package-lock=false".to_string(),
        format!("{}@{}", provider.package(), fixture.exact_version),
    ];
    let mut options = SpawnOptions::new("install Windows provider fixture")
        .current_dir(&prefix)
        .forward_output(false);
    for (key, value) in fixture.launch_env() {
        options = options.env(key, value);
    }
    for key in credential_env_removals() {
        options = options.env_remove(key);
    }
    let output = spawn_logged_blocking_with_deadline(
        &ProcessConsoleHub::new(),
        ProcessKind::AgentBootstrap,
        "npm.cmd",
        &args,
        options,
        Instant::now() + TEST_FIXTURE_INSTALL_TIMEOUT,
    )
    .expect("bounded loopback fixture install");
    assert!(output.success(), "fixture install failed: {output:?}");
    let installed_bin = prefix.join("node_modules").join(".bin");
    // This Windows fixture pins the spawnable .cmd installation entrypoint.
    // Remove npm's POSIX sibling so direct PATH lookup proves that exact path.
    std::fs::remove_file(installed_bin.join(provider.agent_id().command()))
        .expect("remove fixture POSIX shim");
    assert!(installed_bin
        .join(format!("{}.cmd", provider.agent_id().command()))
        .is_file());
    installed_bin
}

fn route_capture_path(root: &Path, route_id: &str) -> PathBuf {
    root.join("route receipts").join(format!("{route_id}.json"))
}

fn assert_canonical_route_manifest() {
    let ids = ROUTE_CASES
        .iter()
        .map(|route| route.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), ROUTE_CASES.len(), "route IDs must be unique");
}

fn runner_file_name_is(command: &str, expected: &str) -> bool {
    Path::new(command)
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(expected))
}
