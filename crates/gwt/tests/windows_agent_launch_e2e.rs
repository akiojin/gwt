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

    // Issue #5102 AC-2: independent launches on the same tree must retain the
    // exact persisted identity assertion on every run, not pass by retrying.
    for repetition in 1..=3 {
        eprintln!("public Board installed identity {provider:?} repetition {repetition}/3");
        assert_public_ws_agent_route_boundary(
            &temp.path().join(format!("public-repeat-{repetition}")),
            provider,
            &fixture,
            &launch_env,
        );
    }

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
    let (fixture, expectation) =
        prepare_public_ws_agent_route(temp_root, provider, npm_fixture, launch_env);
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
    let owned_pid = fixture.pid();
    drop(fixture);
    // A terminated Windows process object can remain in a Toolhelp snapshot
    // while another handle is open. Probe its exit signal, not PID membership.
    assert!(
        !gwt::process::is_host_process_alive(owned_pid),
        "WindowsRealGwtFixture leaked its owned gwt process {owned_pid}"
    );
}

// Keep the public launch setup shared with the startup-restore regression.
// The source is only the Board origin; the runtime under test must be created
// and saved by the production public route, never a hand-written live Session.
fn prepare_public_ws_agent_route(
    temp_root: &Path,
    provider: Provider,
    npm_fixture: &WindowsNpmRegistryFixture,
    launch_env: &std::collections::HashMap<String, String>,
) -> (WindowsRealGwtFixture, PublicRouteExpectation) {
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
    public_env
        .entry("GWT_PHASE75_HOLD_OPEN_MS".to_string())
        .or_insert_with(|| "30000".to_string());
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
    (fixture, expectation)
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
            "public Board route Session did not commit authenticated installed identity: {persisted:?}; receipt={receipt}"
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

/// AC-9 exercises startup admission and the public GUI close path, in addition
/// to the lower-level PendingPane/natural-exit coverage in gwt-terminal.
#[test]
#[ignore = "real Windows gwt/provider/ConPTY startup restore; required Windows provider shards"]
fn windows_startup_restored_provider_tree_closes_through_public_gui() {
    init_test_tracing();
    assert_eq!(required_env(CREDENTIAL_FREE_ENV), "1");
    let provider = Provider::from_env();
    let temp = tempfile::tempdir().expect("Windows restore fixture root");
    let npm = WindowsNpmRegistryFixture::create(temp.path()).expect("loopback provider registry");
    let installed_bin = install_fixture_provider(&npm, provider);
    install_restore_tree_probe(&installed_bin, provider);
    let mut env = launch_env_with_real_gwtd(&npm, &installed_bin);
    env.insert("RUST_LOG".into(), "info,gwt.pane.teardown=info".into());
    env.insert("GWT_PHASE75_HOLD_OPEN_MS".into(), "600000".into());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("restore Tokio runtime");

    let survivor_root = temp.path().join("survivor");
    let (survivor_app, survivor_expectation) =
        prepare_public_ws_agent_route(&survivor_root, provider, &npm, &env);
    let survivor_app = RestoreApp::new(survivor_app);
    let survivor_capture = survivor_expectation.capture.clone();
    let survivor_sessions = survivor_expectation.sessions_dir.clone();
    runtime
        .block_on(assert_public_ws_conpty_boundary_async(survivor_expectation))
        .unwrap_or_else(|error| {
            panic!("survivor public launch: {error}\n{}", survivor_app.stderr())
        });
    let survivor = RestoreTree::capture(&survivor_capture, &survivor_sessions, &survivor_app.root);
    let survivor_receipt = std::fs::read(&survivor_capture).unwrap();
    let survivor_runtime = std::fs::read(&survivor.runtime_path).unwrap();

    let target_root = temp.path().join("restart target");
    let (first_app, first_expectation) =
        prepare_public_ws_agent_route(&target_root, provider, &npm, &env);
    let first_app = RestoreApp::new(first_app);
    let capture = first_expectation.capture.clone();
    let sessions_dir = first_expectation.sessions_dir.clone();
    runtime
        .block_on(assert_public_ws_conpty_boundary_async(first_expectation))
        .unwrap_or_else(|error| panic!("initial public launch: {error}\n{}", first_app.stderr()));
    let first = RestoreTree::capture(&capture, &sessions_dir, &first_app.root);
    assert_ne!(
        first.owner_pid, survivor.owner_pid,
        "fresh distinct projects must have distinct physical owners"
    );
    let original = Session::load(&sessions_dir.join(format!("{}.toml", first.session_id))).unwrap();
    assert!(
        original.restore_window_on_startup,
        "production launch must enable real restore"
    );
    let provider_session_id = original
        .agent_session_id
        .clone()
        .expect("authenticated provider identity");
    assert!(first.processes.iter().all(RestoreProcess::running));
    assert!(survivor.processes.iter().all(RestoreProcess::running));
    assert_eq!(live_restore_gates([&first, &survivor]), 2);

    // Simulate an abrupt app/owner restart. These are captured fixture handles,
    // not a process-name kill or a PID re-open. Persisted windows/Sessions are
    // exclusively the production launch's output; do not synthesize restore.
    let home = first_app.home.clone();
    let workspace = first_app.workspace.clone();
    drop(first_app);
    first.owner.terminate();
    first.assert_tree_exited();
    assert_eq!(live_restore_gates([&first, &survivor]), 1);
    std::fs::remove_file(&capture).unwrap();
    std::fs::remove_file(capture.with_extension("tree.json")).unwrap();

    // Reuse the exact HOME, saved workspace and installed provider. No
    // reopen_recent_project, Board launch, PendingPane or spawn helper is sent
    // after restart: frontend_ready admits the actual startup restore sweep.
    let mut restart_env = env.clone();
    restart_env.insert("GWT_PHASE75_CAPTURE".into(), capture.display().to_string());
    restart_env.insert(
        "GWT_PHASE75_PROVIDER_SESSION_ID".into(),
        provider_session_id.clone(),
    );
    restart_env.insert("GWT_PHASE75_HOLD_OPEN_MS".into(), "600000".into());
    let restored_app = WindowsRealGwtFixture::start(
        &target_root.join("restarted real gwt"),
        Path::new(env!("CARGO_BIN_EXE_gwt")),
        &home,
        &workspace,
        &restart_env,
    )
    .expect("restart real gwt with the production-saved HOME");
    let restored_app = RestoreApp::new(restored_app);
    let restored_evidence = runtime
        .block_on(wait_startup_restore_and_close(
            &restored_app,
            &capture,
            &sessions_dir,
            &original,
            &provider_session_id,
            &survivor,
            &survivor_app,
            &survivor_capture,
            &survivor_receipt,
            &survivor_runtime,
        ))
        .unwrap_or_else(|error| {
            panic!(
                "real startup restore/public close failed: {error}\n{}",
                restored_app.stderr()
            )
        });
    let evidence = PathBuf::from("target/issue-4735-fixforward");
    std::fs::create_dir_all(&evidence).unwrap();
    let evidence = serde_json::json!({
        "test": "windows_startup_restored_provider_tree_closes_through_public_gui",
        "original_session": original.id,
        "provider_session": provider_session_id,
        "original_processes": first.pids,
        "survivor_processes": survivor.pids,
        "restored": restored_evidence,
    });
    eprintln!("Windows startup restore acceptance: {evidence}");
    std::fs::write(
        PathBuf::from("target/issue-4735-fixforward").join(format!(
            "windows-restore-{}.json",
            provider.agent_id().command()
        )),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    // The survivor guard owns the daemon as well as its tree on migrated hosts.
    drop(restored_app);
    drop(survivor_app);
}

fn live_restore_gates<const N: usize>(trees: [&RestoreTree; N]) -> usize {
    trees
        .iter()
        .filter(|tree| tree.processes[0].running())
        .count()
}

/// Add only process observability to the already-installed credential-free
/// provider. Its generated hook execution and provider identity checks remain
/// the same script used by the existing public launch regression.
fn install_restore_tree_probe(installed_bin: &Path, provider: Provider) {
    let script = installed_bin
        .parent()
        .unwrap()
        .join(provider.package())
        .join("bin")
        .join(format!("{}.js", provider.agent_id().command()));
    let original = std::fs::read_to_string(&script).expect("installed provider script");
    assert!(original.contains("const receipt = {"));
    assert!(original
        .contains("Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, holdOpenMs);"));
    let probe = r#"
const treePath = capture.replace(/\.[^.]+$/, '.tree.json');
const descendant = `const cp=require('child_process'),fs=require('fs');
const grandchild=cp.spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'inherit'});
fs.writeFileSync(process.argv[1]+'.tmp',JSON.stringify({target:process.ppid,descendant:process.pid,grandchild:grandchild.pid}));
fs.renameSync(process.argv[1]+'.tmp',process.argv[1]);
setInterval(()=>{},1000);`;
const treeChild = childProcess.spawn(process.execPath, ['-e', descendant, treePath], {stdio:'inherit'});
const treeDeadline = Date.now() + 30000;
while (!fs.existsSync(treePath)) {
  if (Date.now() >= treeDeadline) throw new Error('fixture descendants did not start');
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
}
"#;
    let observed = original.replace("const receipt = {", &format!("{probe}\nconst receipt = {{\n  runtime_path: process.env.GWT_SESSION_RUNTIME_PATH,\n  process_pid: process.pid,\n  process_parent_pid: process.ppid,"));
    // Publish complete fixture receipts, so file existence is the readiness
    // barrier rather than a race against a still-running writeFileSync.
    let observed = observed.replace(
        "fs.writeFileSync(capture, JSON.stringify(receipt));",
        "fs.writeFileSync(capture + '.tmp', JSON.stringify(receipt)); fs.renameSync(capture + '.tmp', capture);",
    );
    let observed = observed.replace(
        "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, holdOpenMs);",
        r#"let probeInput = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => {
  probeInput += chunk;
  let newline;
  while ((newline = probeInput.indexOf('\n')) >= 0) {
    const line = probeInput.slice(0, newline);
    probeInput = probeInput.slice(newline + 1);
    const prefix = 'gwt-restore-probe:';
    const start = line.indexOf(prefix);
    if (start < 0) continue;
    const parts = JSON.parse(line.slice(start + prefix.length).trim());
    console.log('GWT-PROVIDER-REPLY:' + parts[1] + parts[0]);
  }
});
process.stdin.resume();
setTimeout(() => process.exit(0), holdOpenMs);"#,
    );
    std::fs::write(script, observed).expect("install test-local tree probe");
}

// Keep the process object originally returned by CreateProcess as the trust
// root. PID files and second-resolution sysinfo timestamps never authorize a
// destructive cleanup. Unproved candidate handles are closed without signals.
struct RestoreApp {
    app: WindowsRealGwtFixture,
    root: RestoreProcess,
}
impl RestoreApp {
    fn new(app: WindowsRealGwtFixture) -> Self {
        use std::os::windows::io::AsRawHandle;
        let root = RestoreProcess::duplicate_owned(windows::Win32::Foundation::HANDLE(
            app.process_handle().as_raw_handle(),
        ));
        Self { app, root }
    }
}
impl std::ops::Deref for RestoreApp {
    type Target = WindowsRealGwtFixture;
    fn deref(&self) -> &Self::Target {
        &self.app
    }
}
impl Drop for RestoreApp {
    fn drop(&mut self) {
        // Cover failures before a provider receipt exists. Endpoint candidates
        // remain untrusted until their actual process object is proved to be
        // a live direct child of the retained fixture GUI process object.
        let mut pending = vec![self.home.join(".gwt")];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "json") {
                    continue;
                }
                let Ok(bytes) = std::fs::read(path) else {
                    continue;
                };
                let Ok(endpoint) =
                    serde_json::from_slice::<gwt_core::daemon::DaemonEndpoint>(&bytes)
                else {
                    continue;
                };
                if let Ok(owner) = RestoreProcess::capture_child(endpoint.pid, &self.root) {
                    drop(owner);
                }
            }
        }
    }
}

#[repr(C)]
#[derive(Default)]
struct RestoreProcessBasicInformation {
    reserved1: usize,
    peb: usize,
    reserved2: [usize; 2],
    pid: usize,
    parent_pid: usize,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process: windows::Win32::Foundation::HANDLE,
        information_class: u32,
        information: *mut RestoreProcessBasicInformation,
        information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

struct RestoreProcess {
    handle: windows::Win32::Foundation::HANDLE,
    pid: u32,
    created: u64,
    cleanup_authorized: bool,
}
impl RestoreProcess {
    fn inspect(handle: windows::Win32::Foundation::HANDLE) -> Result<Self, String> {
        use windows::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::{GetProcessId, GetProcessTimes},
        };
        let (mut creation, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: query the retained process object, not a second PID lookup.
        if let Err(error) =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
        {
            let _ = unsafe { CloseHandle(handle) };
            return Err(error.to_string());
        }
        Ok(Self {
            handle,
            pid: unsafe { GetProcessId(handle) },
            created: (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime),
            cleanup_authorized: false,
        })
    }
    fn duplicate_owned(handle: windows::Win32::Foundation::HANDLE) -> Self {
        use windows::Win32::{
            Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE},
            System::Threading::GetCurrentProcess,
        };
        let mut duplicate = HANDLE::default();
        // SAFETY: source is the fixture's std::Child or an already proved
        // retained handle. Duplicate preserves that exact process object.
        unsafe {
            let current = GetCurrentProcess();
            DuplicateHandle(
                current,
                handle,
                current,
                &mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
            .unwrap();
        }
        let mut process = Self::inspect(duplicate).expect("query fixture-owned process FILETIME");
        process.cleanup_authorized = true;
        process
    }
    fn capture_child(pid: u32, parent: &Self) -> Result<Self, String> {
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        if pid == std::process::id() || !parent.cleanup_authorized || !parent.running() {
            return Err("fixture parent is not live".into());
        }
        // SAFETY: opening/querying a candidate does not authorize a signal.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                false,
                pid,
            )
        }
        .map_err(|error| error.to_string())?;
        let mut child = Self::inspect(handle)?;
        let mut information = RestoreProcessBasicInformation::default();
        // SAFETY: ProcessBasicInformation writes precisely this C layout.
        let status = unsafe {
            NtQueryInformationProcess(
                child.handle,
                0,
                &mut information,
                std::mem::size_of::<RestoreProcessBasicInformation>() as u32,
                std::ptr::null_mut(),
            )
        };
        if status < 0
            || information.pid != pid as usize
            || information.parent_pid != parent.pid as usize
            || child.created < parent.created
            || !parent.running()
            || !child.running()
        {
            return Err(format!(
                "candidate {pid} lacks exact live fixture-parent/FILETIME proof"
            ));
        }
        // The parent HANDLE is still live: its PID cannot have been reused.
        // Its exact FILETIME predates the child's, rejecting a stale parent
        // number inherited by a process born before this parent incarnation.
        child.cleanup_authorized = true;
        Ok(child)
    }
    fn running(&self) -> bool {
        // SAFETY: this exact process object remains retained until Drop.
        unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(self.handle, 0)
                == windows::Win32::Foundation::WAIT_TIMEOUT
        }
    }
    fn terminate(&self) {
        assert!(
            self.cleanup_authorized,
            "unproved process cannot be signalled"
        );
        if self.running() {
            // SAFETY: only std::Child-rooted, FILETIME/lineage-proved handles.
            unsafe { windows::Win32::System::Threading::TerminateProcess(self.handle, 91) }
                .unwrap();
        }
    }
    fn assert_exited_by(&self, deadline: Instant) {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u32;
        assert_eq!(
            unsafe {
                windows::Win32::System::Threading::WaitForSingleObject(self.handle, remaining)
            },
            windows::Win32::Foundation::WAIT_OBJECT_0,
            "fixture tree process leaked"
        );
    }
}
impl Drop for RestoreProcess {
    fn drop(&mut self) {
        // A rejected candidate reaches this branch with cleanup_authorized
        // false: never signal a process merely named by a receipt/endpoint.
        if self.cleanup_authorized && self.running() {
            let _ = unsafe { windows::Win32::System::Threading::TerminateProcess(self.handle, 91) };
            let _ = unsafe {
                windows::Win32::System::Threading::WaitForSingleObject(self.handle, 5000)
            };
        }
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.handle) };
    }
}

struct RestoreTree {
    processes: Vec<RestoreProcess>, // gate, optional cmd wrapper, provider and descendants
    pids: Vec<u32>,
    owner: RestoreProcess,
    owner_pid: u32,
    session_id: String,
    runtime_path: PathBuf,
}
impl RestoreTree {
    fn capture(capture: &Path, sessions_dir: &Path, root: &RestoreProcess) -> Self {
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
        let runtime_path = PathBuf::from(
            receipt["runtime_path"]
                .as_str()
                .expect("full runtime sidecar path"),
        );
        assert!(
            runtime_path.starts_with(sessions_dir),
            "runtime outside fixture HOME"
        );
        let state: gwt_agent::SessionRuntimeState =
            serde_json::from_slice(&std::fs::read(&runtime_path).unwrap()).unwrap();
        let host_pid: u32 = runtime_path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            gwt::process::host_process_start_time(host_pid),
            state.host_started_at
        );
        assert!(
            state.host_started_at.is_some(),
            "missing exact owner birth proof"
        );
        let owner = if host_pid == root.pid {
            let owner = RestoreProcess::duplicate_owned(root.handle);
            assert_eq!(
                owner.created, root.created,
                "same-owner HANDLE must preserve exact FILETIME"
            );
            owner
        } else {
            RestoreProcess::capture_child(host_pid, root)
                .expect("owner must descend from exact fixture GUI")
        };
        let gate_pid = state.child_pid.expect("production PTY gate identity");
        assert_eq!(
            gwt::process::host_process_start_time(gate_pid),
            state.child_started_at
        );
        assert!(
            state.child_started_at.is_some(),
            "missing exact gate birth proof"
        );
        let gate = RestoreProcess::capture_child(gate_pid, &owner)
            .expect("gate must descend from exact owner");
        let parent_pid = receipt["process_parent_pid"]
            .as_u64()
            .expect("provider parent PID") as u32;
        let mut processes = vec![gate];
        if parent_pid != gate_pid {
            processes.push(
                RestoreProcess::capture_child(parent_pid, &processes[0])
                    .expect("cmd wrapper must descend from exact gate"),
            );
        }
        let tree: serde_json::Value =
            serde_json::from_slice(&std::fs::read(capture.with_extension("tree.json")).unwrap())
                .unwrap();
        assert_eq!(tree["target"], receipt["process_pid"]);
        for role in ["target", "descendant", "grandchild"] {
            let pid = tree[role].as_u64().unwrap() as u32;
            let parent = processes.last().unwrap();
            processes.push(
                RestoreProcess::capture_child(pid, parent)
                    .unwrap_or_else(|error| panic!("{role}: {error}")),
            );
        }
        let pids = processes.iter().map(|process| process.pid).collect();
        assert!(processes.iter().all(RestoreProcess::running));
        Self {
            processes,
            pids,
            owner,
            owner_pid: host_pid,
            session_id: receipt["gwt_session_id"].as_str().unwrap().into(),
            runtime_path,
        }
    }
    fn assert_tree_exited(&self) {
        // test-hygiene: allow-wall-clock-deadline real captured Windows process handles prove tree exit; this deadline only bounds an OS process-response hang
        let deadline = Instant::now() + Duration::from_secs(30);
        for process in &self.processes {
            process.assert_exited_by(deadline);
        }
    }
}

type RestoreSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn restore_send(socket: &mut RestoreSocket, value: serde_json::Value) -> Result<(), String> {
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            value.to_string().into(),
        ))
        .await
        .map_err(|error| format!("public restore send: {error}"))
}

async fn restore_event(socket: &mut RestoreSocket) -> Result<Option<serde_json::Value>, String> {
    // test-hygiene: allow-wall-clock-deadline real WebSocket receive must yield periodically so the outer actual-process deadline remains bounded
    match tokio::time::timeout(Duration::from_millis(500), socket.next()).await {
        Err(_) => Ok(None),
        Ok(None) => Err("public restore websocket closed".into()),
        Ok(Some(Err(error))) => Err(format!("public restore receive: {error}")),
        Ok(Some(Ok(message))) => Ok(message
            .to_text()
            .ok()
            .and_then(|text| serde_json::from_str(text).ok())),
    }
}

async fn connect_saved_restore_project(
    app: &WindowsRealGwtFixture,
) -> Result<RestoreSocket, String> {
    let url = app.public_ws_url();
    let (mut hub, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|error| format!("connect restored Hub: {error}"))?;
    // test-hygiene: allow-wall-clock-deadline real authenticated project WebSocket startup and saved-root discovery depend on OS listener readiness; the deadline is a hang guard
    let deadline = Instant::now() + Duration::from_secs(60);
    let key = loop {
        // test-hygiene: allow-wall-clock-deadline real authenticated project WebSocket startup and saved-root discovery depend on OS listener readiness; the deadline is a hang guard
        if Instant::now() >= deadline {
            return Err("saved project never appeared at startup".into());
        }
        restore_send(&mut hub, serde_json::json!({"kind":"frontend_ready"})).await?;
        if let Some(event) = restore_event(&mut hub).await? {
            if event["kind"] == "hub_state" {
                let keys = hub_project_keys(&event);
                if keys.len() == 1 {
                    break keys[0].clone();
                }
            }
        }
    };
    hub.close(None).await.map_err(|error| error.to_string())?;
    let mut project_url = reqwest::Url::parse(&url).map_err(|error| error.to_string())?;
    project_url.query_pairs_mut().append_pair("repo_hash", &key);
    let (mut socket, _) = tokio_tungstenite::connect_async(project_url.as_str())
        .await
        .map_err(|error| format!("connect restored project: {error}"))?;
    restore_send(&mut socket, serde_json::json!({"kind":"frontend_ready"})).await?;
    Ok(socket)
}

fn restore_live_agent_count(windows: &[serde_json::Value]) -> usize {
    windows
        .iter()
        .filter(|window| {
            window["session_id"].is_string()
                && matches!(
                    window["status"].as_str(),
                    Some("running" | "waiting" | "idle")
                )
        })
        .count()
}

#[allow(clippy::too_many_arguments)]
async fn wait_startup_restore_and_close(
    app: &RestoreApp,
    capture: &Path,
    sessions_dir: &Path,
    original: &Session,
    provider_session_id: &str,
    survivor: &RestoreTree,
    survivor_app: &WindowsRealGwtFixture,
    survivor_capture: &Path,
    survivor_receipt: &[u8],
    survivor_runtime: &[u8],
) -> Result<serde_json::Value, String> {
    let mut socket = connect_saved_restore_project(app).await?;
    // test-hygiene: allow-wall-clock-deadline real provider capture, restored projection, and exact finalizer join receipts establish ordering; the deadline only bounds OS and IPC delivery
    let deadline = Instant::now() + TEST_AGENT_READY_TIMEOUT;
    let mut windows = Vec::new();
    // test-hygiene: allow-wall-clock-deadline real provider capture, restored projection, and exact finalizer join receipts establish ordering; the deadline only bounds OS and IPC delivery
    while !capture.is_file() && Instant::now() < deadline {
        restore_send(&mut socket, serde_json::json!({"kind":"list_windows"})).await?;
        if let Some(event) = restore_event(&mut socket).await? {
            if event["kind"] == "launch_error" {
                return Err(format!("startup restore failed: {event}"));
            }
            if event["kind"] == "window_list" {
                windows = event["windows"].as_array().cloned().unwrap_or_default();
            }
            if matches!(
                event["kind"].as_str(),
                Some("terminal_output" | "terminal_snapshot")
            ) {
                if let Some(encoded) = event["data_base64"].as_str() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|error| error.to_string())?;
                    if bytes.windows(4).any(|part| part == b"\x1b[6n") {
                        restore_send(
                            &mut socket,
                            serde_json::json!({
                                "kind":"terminal_input", "id":event["id"], "data":"\u{1b}[1;1R"
                            }),
                        )
                        .await?;
                    }
                }
            }
        }
    }
    if !capture.is_file() {
        return Err("startup restore never invoked the provider".into());
    }
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(capture).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let restored = RestoreTree::capture(capture, sessions_dir, &app.root);
    assert_ne!(
        restored.owner_pid, survivor.owner_pid,
        "restored project must retain a separate physical owner"
    );
    let session = Session::load(&sessions_dir.join(format!("{}.toml", restored.session_id)))
        .map_err(|error| error.to_string())?;
    if session.restore_source_session_id.as_deref() != Some(original.id.as_str())
        || session.agent_session_id.as_deref() != Some(provider_session_id)
        || receipt["provider_session_id"] != provider_session_id
        || receipt["hook_status"] != 0
        || receipt["hook_forward_token_present"] != true
        || receipt["tty"] != true
        || receipt["package"] != Provider::from_env().package()
        || receipt["version"].as_str() != original.tool_version.as_deref()
        || receipt["npm_exec_identity"] != serde_json::Value::Null
        || session.tool_runtime_provenance.is_some()
        || session.tool_version != original.tool_version
    {
        return Err(format!(
            "startup restore lost exact authenticated lineage: {session:?}; {receipt}"
        ));
    }
    let resume_flag = if original.agent_id == AgentId::Codex {
        "resume"
    } else {
        "--resume"
    };
    let argv = receipt["argv"]
        .as_array()
        .ok_or("restore receipt omitted argv")?;
    if !argv
        .windows(2)
        .any(|pair| pair[0] == resume_flag && pair[1] == provider_session_id)
    {
        return Err(format!(
            "startup did not resume the exact original provider conversation: {argv:?}"
        ));
    }
    if session.id == original.id {
        return Err("restore must publish its exact predecessor lineage".into());
    }
    let window_id = loop {
        if let Some(window) = windows
            .iter()
            .find(|window| window["session_id"] == session.id)
        {
            if restore_live_agent_count(&windows) == 1 {
                break window["id"]
                    .as_str()
                    .ok_or("restored window missing id")?
                    .to_string();
            }
        }
        // test-hygiene: allow-wall-clock-deadline real provider capture, restored projection, and exact finalizer join receipts establish ordering; the deadline only bounds OS and IPC delivery
        if Instant::now() >= deadline {
            return Err(format!("restored live pane not projected: {windows:?}"));
        }
        restore_send(&mut socket, serde_json::json!({"kind":"list_windows"})).await?;
        if let Some(event) = restore_event(&mut socket).await? {
            if event["kind"] == "window_list" {
                windows = event["windows"].as_array().cloned().unwrap_or_default();
            }
        }
    };
    assert_eq!(
        live_restore_gates([&restored, survivor]),
        2,
        "two live gated agent panes before close"
    );
    let owner_client = gwt::project_runtime::client::ProjectRuntimeClient::from_current_project(
        &app.workspace,
        &app.home.join(".gwt"),
    )
    .await?;
    let owner_identity = owner_client
        .identity()
        .ok_or("restored owner identity missing")?;
    if owner_identity.pid != restored.owner_pid {
        return Err("restored owner changed before public close".into());
    }
    let runtime_state = gwt_agent::SessionRuntimeState::load(&restored.runtime_path)
        .map_err(|error| error.to_string())?;
    if runtime_state.host_started_at != Some(owner_identity.host_started_at) {
        return Err("restored owner birth changed before public close".into());
    }
    let inventory = owner_client
        .command(gwt::project_runtime::events::OwnerCommand::ListPanes)
        .await?;
    let panes = inventory["panes"]
        .as_array()
        .ok_or("restored owner inventory omitted panes")?;
    let exact = panes
        .iter()
        .filter(|pane| pane["session_id"] == session.id)
        .collect::<Vec<_>>();
    let [exact] = exact.as_slice() else {
        return Err("restored Session has no unique owner pane".into());
    };
    let owner_pane_id = exact["id"]
        .as_str()
        .ok_or("restored owner pane id missing")?
        .to_string();
    let owner_incarnation = exact["incarnation"]
        .as_u64()
        .ok_or("restored owner incarnation missing")?;
    if runtime_state.runtime_incarnation != Some(owner_incarnation) {
        return Err("restored owner incarnation changed before public close".into());
    }
    restore_send(
        &mut socket,
        serde_json::json!({"kind":"close_window", "id":window_id}),
    )
    .await?;
    // test-hygiene: allow-wall-clock-deadline real provider capture, restored projection, and exact finalizer join receipts establish ordering; the deadline only bounds OS and IPC delivery
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut removed = false;
    loop {
        restore_send(&mut socket, serde_json::json!({"kind":"list_windows"})).await?;
        if let Some(event) = restore_event(&mut socket).await? {
            if event["kind"] == "window_list" {
                let windows = event["windows"]
                    .as_array()
                    .ok_or("window_list omitted windows")?;
                removed = !windows.iter().any(|window| window["id"] == window_id)
                    && restore_live_agent_count(windows) == 0;
            }
        }
        if removed && restore_finalizer_join_completed(app, &owner_pane_id, owner_incarnation) {
            break;
        }
        // test-hygiene: allow-wall-clock-deadline real provider capture, restored projection, and exact finalizer join receipts establish ordering; the deadline only bounds OS and IPC delivery
        if Instant::now() >= deadline {
            return Err(format!("public close failed to remove pane and complete reader EOF/join: removed={removed}, id={window_id}"));
        }
    }
    restored.assert_tree_exited();
    assert_eq!(
        live_restore_gates([&restored, survivor]),
        1,
        "only survivor's agent gate may remain"
    );
    assert!(survivor.processes.iter().all(RestoreProcess::running));
    assert_eq!(
        std::fs::read(survivor_capture).unwrap(),
        survivor_receipt,
        "survivor must not relaunch"
    );
    let before: gwt_agent::SessionRuntimeState = serde_json::from_slice(survivor_runtime).unwrap();
    let after: gwt_agent::SessionRuntimeState =
        serde_json::from_slice(&std::fs::read(&survivor.runtime_path).unwrap()).unwrap();
    assert_eq!(
        (
            after.execution_identity,
            after.runtime_incarnation,
            after.child_pid,
            after.child_started_at,
            after.host_started_at
        ),
        (
            before.execution_identity,
            before.runtime_incarnation,
            before.child_pid,
            before.child_started_at,
            before.host_started_at
        ),
        "target close must preserve survivor Session/incarnation/exact OS identities"
    );
    assert_restore_survivor_io(survivor_app, &survivor.session_id).await?;
    Ok(serde_json::json!({
        "window_id": window_id,
        "owner_pane_id": owner_pane_id,
        "owner_incarnation": owner_incarnation,
        "session_id": session.id,
        "processes": restored.pids,
        "public_close": "completed",
        "reader_eof_join": "completed",
        "live_agent_panes": 1,
        "live_agent_gates": 1,
        "survivor_io": "pass",
    }))
}

async fn assert_restore_survivor_io(
    app: &WindowsRealGwtFixture,
    session_id: &str,
) -> Result<(), String> {
    let mut socket = connect_saved_restore_project(app).await?;
    // test-hygiene: allow-wall-clock-deadline real survivor shell and PTY output markers establish progress after close; the deadline only bounds OS and IPC delivery
    let deadline = Instant::now() + Duration::from_secs(30);
    let provider_window = loop {
        // test-hygiene: allow-wall-clock-deadline real survivor shell and PTY output markers establish progress after close; the deadline only bounds OS and IPC delivery
        if Instant::now() >= deadline {
            return Err("survivor live pane census timed out".into());
        }
        restore_send(&mut socket, serde_json::json!({"kind":"list_windows"})).await?;
        if let Some(event) = restore_event(&mut socket).await? {
            if event["kind"] == "window_list" {
                let windows = event["windows"]
                    .as_array()
                    .ok_or("survivor windows missing")?;
                if restore_live_agent_count(windows) != 1 {
                    return Err("survivor gated agent pane count changed".into());
                }
                if let Some(provider) = windows
                    .iter()
                    .find(|window| window["session_id"] == session_id)
                {
                    break provider["id"]
                        .as_str()
                        .ok_or("survivor exact provider window id missing")?
                        .to_string();
                }
            }
        }
    };
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let (left, right) = nonce.split_at(16);
    let input = format!("gwt-restore-probe:{}\r", serde_json::json!([left, right]));
    let marker = format!("GWT-PROVIDER-REPLY:{right}{left}");
    assert!(
        !input.contains(&marker),
        "PTY input echo must never satisfy the provider response"
    );
    restore_send(
        &mut socket,
        serde_json::json!({"kind":"terminal_input", "id":provider_window, "data":input}),
    )
    .await?;
    let mut output = String::new();
    // test-hygiene: allow-wall-clock-deadline real survivor shell and PTY output markers establish progress after close; the deadline only bounds OS and IPC delivery
    while Instant::now() < deadline {
        if let Some(event) = restore_event(&mut socket).await? {
            if event["id"] == provider_window && event["kind"] == "terminal_output" {
                if let Some(encoded) = event["data_base64"].as_str() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|error| error.to_string())?;
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    if output.contains(&marker) {
                        return Ok(());
                    }
                }
            }
        }
    }
    Err(format!(
        "survivor lost public input/output after target close: {output:?}"
    ))
}

/// This event is emitted only after release_descriptors and successful output
/// and status joins. terminal_status alone is intentionally insufficient: its
/// independent status watcher can publish exit before the reader reaches EOF.
fn restore_finalizer_join_completed(
    app: &WindowsRealGwtFixture,
    owner_pane_id: &str,
    incarnation: u64,
) -> bool {
    let mut logs = vec![app.stderr()];
    let mut pending = vec![app.home.join(".gwt")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "log") {
                if let Ok(log) = std::fs::read_to_string(path) {
                    logs.push(log);
                }
            }
        }
    }
    logs.iter().any(|log| {
        log.lines().any(|line| {
            // Strip tracing's ANSI decoration before inspecting field values.
            let mut plain = String::new();
            let mut escape = false;
            for character in line.chars() {
                if character == '\u{1b}' {
                    escape = true;
                } else if escape {
                    if character == 'm' {
                        escape = false;
                    }
                } else {
                    plain.push(character);
                }
            }
            let incarnation_matches = serde_json::from_str::<serde_json::Value>(&plain)
                .ok()
                .is_some_and(|event| {
                    event.get("fields").unwrap_or(&event)["incarnation"] == incarnation
                })
                || plain.split_whitespace().any(|field| {
                    let field = field.trim_end_matches(',');
                    field == format!("incarnation={incarnation}")
                        || field == format!("incarnation=\"{incarnation}\"")
                });
            plain.contains(owner_pane_id)
                && incarnation_matches
                && plain.contains("close_finalizer")
                && (plain.contains("outcome=completed")
                    || plain.contains("outcome=\"completed\"")
                    || plain.contains("\"outcome\":\"completed\""))
        })
    })
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
