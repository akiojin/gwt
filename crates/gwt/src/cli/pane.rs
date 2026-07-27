//! `pane.*` JSON operations for live agent-pane inspection.

use std::{collections::HashMap, path::Path, time::Duration};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use gwt_agent::{session::GWT_SESSION_ID_ENV, GWT_PANE_WS_URL_ENV};
use gwt_github::{ApiError, SpecOpsError};
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::AUTHORIZATION, HeaderValue},
        Message,
    },
};

use crate::{
    persistence::{PersistedWindowState, WindowState},
    preset::WindowPreset,
};

#[cfg(test)]
use crate::persistence::WindowLaneKind;
#[cfg(test)]
use crate::persistence::WindowPlacement;

use super::{CliEnv, CliParseError, PaneCommand};

const DEFAULT_READ_LINES: usize = 50;
const PROJECT_ROOT_ENV: &str = "GWT_PROJECT_ROOT";

type PaneWebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub fn parse(args: &[String]) -> Result<PaneCommand, CliParseError> {
    let Some((head, rest)) = args.split_first() else {
        return Ok(PaneCommand::List);
    };

    match head.as_str() {
        "list" => {
            ensure_no_args(rest)?;
            Ok(PaneCommand::List)
        }
        "read" => {
            let (id, rest) = rest.split_first().ok_or(CliParseError::Usage)?;
            Ok(PaneCommand::Read {
                id: id.clone(),
                lines: parse_lines(rest)?,
            })
        }
        "close" | "stop" => {
            let (id, rest) = rest.split_first().ok_or(CliParseError::Usage)?;
            ensure_no_args(rest)?;
            Ok(PaneCommand::Close { id: id.clone() })
        }
        "send" => {
            let (id, text) = parse_send_args(rest)?;
            Ok(PaneCommand::Send { id, text })
        }
        id => Ok(PaneCommand::Read {
            id: id.to_string(),
            lines: parse_lines(rest)?,
        }),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: PaneCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let ws_url = pane_websocket_url_from_env().map_err(config_error)?;
    let project_root = project_root_for_pane(env.repo_path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| config_error(format!("failed to create pane runtime: {err}")))?;

    let output = runtime
        .block_on(run_async(&ws_url, &project_root, command))
        .map_err(config_error)?;
    out.push_str(&output);
    Ok(0)
}

async fn run_async(
    ws_url: &str,
    project_root: &str,
    command: PaneCommand,
) -> Result<String, String> {
    match command {
        PaneCommand::List => {
            let windows = request_window_list(ws_url, project_root).await?;
            Ok(render_pane_list(&windows))
        }
        PaneCommand::Read { id, lines } => {
            read_pane_snapshot(ws_url, project_root, &id, lines).await
        }
        PaneCommand::Close { id } => close_pane(ws_url, project_root, &id).await,
        PaneCommand::Send { id, text } => {
            send_pane_input(ws_url, project_root, id.as_deref(), &text).await
        }
    }
}

/// SPEC-3050 FR-001/FR-002: queue one line into the calling agent's own pane.
/// The injected line is submitted by the runtime once the agent's current
/// turn ends, which is what the gwt-discussion "Goal Start" step relies on.
async fn send_pane_input(
    ws_url: &str,
    project_root: &str,
    requested_id: Option<&str>,
    text: &str,
) -> Result<String, String> {
    let session_id = std::env::var(GWT_SESSION_ID_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "{GWT_SESSION_ID_ENV} is not set; pane.send injects only into the calling agent's own pane"
            )
        })?;

    let windows = request_window_list(ws_url, project_root).await?;
    let window_id = resolve_send_target(&windows, requested_id, &session_id)?;
    let line = ensure_trailing_submit(text);

    let mut socket = connect_pane_websocket(ws_url).await?;
    send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;
    let scoped_windows = next_workspace_windows(&mut socket, project_root, "pane send").await?;
    let scoped_window_id = resolve_send_target(&scoped_windows, requested_id, &session_id)?;
    if scoped_window_id != window_id {
        return Err("pane send target changed while establishing the authenticated scope".into());
    }
    send_frontend_event(
        &mut socket,
        json!({ "kind": "pane_send_input", "session_id": session_id, "text": line }),
    )
    .await?;

    for _ in 0..128 {
        let value = next_backend_json(&mut socket).await?;
        let Some(reply) = parse_pane_send_result(&value)? else {
            continue;
        };
        return if reply.ok {
            Ok(format!(
                "sent input to {}\n",
                reply.window_id.unwrap_or(window_id)
            ))
        } else {
            Err(format!(
                "pane send rejected: {}",
                reply.error.unwrap_or_else(|| "unknown error".to_string())
            ))
        };
    }
    Err("pane send: backend did not return pane_send_result".to_string())
}

async fn request_window_list(
    ws_url: &str,
    project_root: &str,
) -> Result<Vec<PersistedWindowState>, String> {
    let mut socket = connect_pane_websocket(ws_url).await?;
    send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;

    next_workspace_windows(&mut socket, project_root, "pane list").await
}

async fn read_pane_snapshot(
    ws_url: &str,
    project_root: &str,
    requested_id: &str,
    lines: usize,
) -> Result<String, String> {
    let mut socket = connect_pane_websocket(ws_url).await?;
    send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;

    let mut windows = Vec::new();
    let mut snapshots = HashMap::<String, String>::new();

    for _ in 0..128 {
        let value = next_backend_json(&mut socket).await?;
        if let Some(mut parsed) = parse_workspace_windows(&value, project_root) {
            windows.append(&mut parsed);
        }
        if let Some((id, snapshot)) = parse_terminal_snapshot(&value)? {
            snapshots.insert(id, snapshot);
        }

        let resolved_id = resolve_window_id(&windows, requested_id).unwrap_or(requested_id);
        if let Some(snapshot) = snapshots.get(resolved_id) {
            return Ok(render_snapshot_lines(snapshot, lines));
        }
    }

    let known = windows
        .iter()
        .map(|window| window.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(if known.is_empty() {
        format!("pane read: no snapshot received for {requested_id}")
    } else {
        format!("pane read: no snapshot received for {requested_id}; known panes: {known}")
    })
}

async fn close_pane(
    ws_url: &str,
    project_root: &str,
    requested_id: &str,
) -> Result<String, String> {
    let windows = request_window_list(ws_url, project_root).await?;
    let Some(resolved_id) = resolve_window_id(&windows, requested_id).map(str::to_string) else {
        return Err(format!("pane close: unknown pane {requested_id}"));
    };

    let mut socket = connect_pane_websocket(ws_url).await?;
    send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;
    let scoped_windows = next_workspace_windows(&mut socket, project_root, "pane close").await?;
    if resolve_window_id(&scoped_windows, &resolved_id) != Some(resolved_id.as_str()) {
        return Err(format!(
            "pane close: pane {requested_id} left the authenticated project scope"
        ));
    }
    let ambient_session_id = std::env::var(GWT_SESSION_ID_ENV).ok();
    let ambient_session_id = ambient_session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty());
    let closes_calling_session = ambient_session_id.is_some_and(|session_id| {
        scoped_windows.iter().any(|window| {
            window.id == resolved_id && window.session_id.as_deref() == Some(session_id)
        })
    });
    let close_request_id = closes_calling_session.then(|| uuid::Uuid::new_v4().to_string());
    let close_event = match &close_request_id {
        Some(request_id) => {
            json!({ "kind": "close_window", "id": resolved_id, "request_id": request_id })
        }
        None => json!({ "kind": "close_window", "id": resolved_id }),
    };
    send_frontend_event(&mut socket, close_event).await?;
    if let Some(request_id) = close_request_id.as_deref() {
        wait_for_pane_close_acceptance(
            &mut socket,
            request_id,
            &resolved_id,
            Duration::from_secs(2),
        )
        .await?;
        return Ok(format!("close requested {requested_id}\n"));
    }
    send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;

    let windows = next_workspace_windows(&mut socket, project_root, "pane close").await?;
    if resolve_window_id(&windows, &resolved_id).is_none() {
        Ok(format!("closed {requested_id}\n"))
    } else {
        Err(format!(
            "pane close: backend did not close {requested_id}; the target may be this authenticated Session and requires a correlated acceptance"
        ))
    }
}

async fn connect_pane_websocket(ws_url: &str) -> Result<PaneWebSocket, String> {
    let request = pane_websocket_request(ws_url)?;
    connect_async(request)
        .await
        .map(|(socket, _)| socket)
        .map_err(|err| format!("pane websocket connect failed ({ws_url}): {err}"))
}

fn pane_websocket_request(
    ws_url: &str,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
    let mut request = ws_url
        .into_client_request()
        .map_err(|_| "invalid pane WebSocket URL".to_string())?;
    if !matches!(request.uri().scheme_str(), Some("ws" | "wss")) || request.uri().query().is_some()
    {
        return Err("pane WebSocket URL must use ws/wss without a query".to_string());
    }
    if request.uri().path() != "/internal/pane-ws" {
        return Err("pane WebSocket URL must use the exact /internal/pane-ws path".to_string());
    }

    let token = std::env::var(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            format!(
                "{} is not set; relaunch the Session from gwt before using pane.*",
                gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV
            )
        })?;
    let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "invalid pane capability token".to_string())?;
    request.headers_mut().insert(AUTHORIZATION, bearer);
    Ok(request)
}

async fn send_frontend_event(socket: &mut PaneWebSocket, payload: Value) -> Result<(), String> {
    socket
        .send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|err| format!("pane websocket send failed: {err}"))
}

async fn next_backend_json(socket: &mut PaneWebSocket) -> Result<Value, String> {
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .map_err(|_| "pane websocket timed out waiting for backend response".to_string())?
        .ok_or_else(|| "pane websocket closed before backend response".to_string())?
        .map_err(|err| format!("pane websocket receive failed: {err}"))?;

    match message {
        Message::Text(text) => serde_json::from_str(text.as_ref())
            .map_err(|err| format!("pane backend returned invalid JSON: {err}")),
        Message::Binary(bytes) => serde_json::from_slice(&bytes)
            .map_err(|err| format!("pane backend returned invalid JSON: {err}")),
        other => Err(format!(
            "pane backend returned unsupported websocket message: {other:?}"
        )),
    }
}

async fn next_workspace_windows(
    socket: &mut PaneWebSocket,
    project_root: &str,
    context: &str,
) -> Result<Vec<PersistedWindowState>, String> {
    for _ in 0..32 {
        let value = next_backend_json(socket).await?;
        if let Some(windows) = parse_workspace_windows(&value, project_root) {
            return Ok(windows);
        }
    }
    Err(format!("{context}: backend did not return workspace_state"))
}

fn parse_send_args(args: &[String]) -> Result<(Option<String>, String), CliParseError> {
    match args {
        [flag, text] if flag == "--text" => Ok((None, text.clone())),
        [id, flag, text] if flag == "--text" => Ok((Some(id.clone()), text.clone())),
        _ => Err(CliParseError::Usage),
    }
}

/// SPEC-3050 FR-002: the send target is always the caller's own pane. An
/// explicit pane id is accepted only when it resolves to the window bound to
/// the caller's `GWT_SESSION_ID`; everything else is rejected client-side
/// (the server re-checks by resolving the session id itself).
fn resolve_send_target(
    windows: &[PersistedWindowState],
    requested_id: Option<&str>,
    session_id: &str,
) -> Result<String, String> {
    let own = windows
        .iter()
        .find(|window| window.session_id.as_deref() == Some(session_id));
    match requested_id {
        Some(requested) => {
            let Some(resolved) = resolve_window_id(windows, requested) else {
                return Err(format!("pane send: unknown pane {requested}"));
            };
            match own {
                Some(own_window) if own_window.id == resolved => Ok(resolved.to_string()),
                _ => Err(format!(
                    "pane send: pane {requested} is not bound to this session (self-only injection)"
                )),
            }
        }
        None => own.map(|window| window.id.clone()).ok_or_else(|| {
            format!("pane send: no pane is bound to session {session_id} (self-only injection)")
        }),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PaneSendReply {
    ok: bool,
    window_id: Option<String>,
    error: Option<String>,
}

fn parse_pane_close_acceptance(
    value: &Value,
    expected_request_id: &str,
    expected_window_id: &str,
) -> Result<bool, String> {
    if value.get("kind").and_then(Value::as_str) != Some("pane_close_accepted") {
        return Ok(false);
    }
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "pane_close_accepted missing request_id".to_string())?;
    let window_id = value
        .get("window_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "pane_close_accepted missing window_id".to_string())?;
    Ok(request_id == expected_request_id && window_id == expected_window_id)
}

async fn wait_for_pane_close_acceptance(
    socket: &mut PaneWebSocket,
    expected_request_id: &str,
    expected_window_id: &str,
    deadline_after: Duration,
) -> Result<(), String> {
    tokio::time::timeout(deadline_after, async {
        loop {
            let value = next_backend_json(socket).await?;
            if parse_pane_close_acceptance(&value, expected_request_id, expected_window_id)? {
                return Ok::<(), String>(());
            }
        }
    })
    .await
    .map_err(|_| "pane close: backend timed out before matching pane_close_accepted".to_string())?
}

fn parse_pane_send_result(value: &Value) -> Result<Option<PaneSendReply>, String> {
    if value.get("kind").and_then(Value::as_str) != Some("pane_send_result") {
        return Ok(None);
    }
    let ok = value
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| "pane_send_result missing ok".to_string())?;
    let window_id = value
        .get("window_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let error = value
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(Some(PaneSendReply {
        ok,
        window_id,
        error,
    }))
}

/// The injected text must end with a submit key so the runtime actually
/// queues the line instead of leaving it in the composer.
fn ensure_trailing_submit(text: &str) -> String {
    if text.ends_with('\r') || text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\r")
    }
}

fn parse_lines(args: &[String]) -> Result<usize, CliParseError> {
    if args.is_empty() {
        return Ok(DEFAULT_READ_LINES);
    }
    if args.len() != 2 || args[0] != "--lines" {
        return Err(CliParseError::Usage);
    }
    args[1]
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(args[1].clone()))
}

fn ensure_no_args(args: &[String]) -> Result<(), CliParseError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(CliParseError::Usage)
    }
}

fn pane_websocket_url_from_env() -> Result<String, String> {
    std::env::var(GWT_PANE_WS_URL_ENV)
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            format!(
                "{GWT_PANE_WS_URL_ENV} is not set; relaunch the Session from gwt before using pane.*"
            )
        })
}

fn project_root_for_pane(default: &Path) -> String {
    std::env::var(PROJECT_ROOT_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string_lossy().into_owned())
}

fn parse_workspace_windows(value: &Value, project_root: &str) -> Option<Vec<PersistedWindowState>> {
    if value.get("kind")?.as_str()? != "workspace_state" {
        return None;
    }
    let tabs = value.get("workspace")?.get("tabs")?.as_array()?;
    let mut matching_windows = Vec::new();
    let mut fallback_windows = Vec::new();
    let mut matched_project = false;
    for tab in tabs {
        let Some(tab_windows) = tab
            .get("workspace")
            .and_then(|workspace| workspace.get("windows"))
        else {
            continue;
        };
        if let Ok(mut parsed) =
            serde_json::from_value::<Vec<PersistedWindowState>>(tab_windows.clone())
        {
            if tab.get("project_root").and_then(Value::as_str) == Some(project_root) {
                matched_project = true;
                matching_windows.append(&mut parsed);
            } else {
                fallback_windows.append(&mut parsed);
            }
        }
    }
    if matched_project {
        Some(matching_windows)
    } else {
        Some(fallback_windows)
    }
}

fn parse_terminal_snapshot(value: &Value) -> Result<Option<(String, String)>, String> {
    if value.get("kind").and_then(Value::as_str) != Some("terminal_snapshot") {
        return Ok(None);
    }
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "terminal_snapshot missing id".to_string())?
        .to_string();
    let data = value
        .get("data_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| "terminal_snapshot missing data_base64".to_string())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|err| format!("terminal_snapshot base64 decode failed: {err}"))?;
    let text = String::from_utf8_lossy(&decoded).to_string();
    Ok(Some((id, text)))
}

fn render_snapshot_lines(snapshot: &str, lines: usize) -> String {
    let mut selected = snapshot.lines().rev().take(lines).collect::<Vec<_>>();
    selected.reverse();
    let mut out = selected.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub(crate) fn render_pane_list(windows: &[PersistedWindowState]) -> String {
    let panes = windows.iter().filter(|window| is_agent_pane(window));
    let mut out = String::new();
    for window in panes {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            window.id,
            status_label(window.status),
            window
                .agent_id
                .as_deref()
                .unwrap_or_else(|| preset_label(window.preset)),
            window
                .dynamic_title
                .as_deref()
                .or(window.purpose_title.as_deref())
                .unwrap_or(&window.title)
        ));
    }
    if out.is_empty() {
        out.push_str("no active agent panes\n");
    }
    out
}

fn is_agent_pane(window: &PersistedWindowState) -> bool {
    window.agent_id.is_some()
        || matches!(
            window.preset,
            WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex
        )
}

fn resolve_window_id<'a>(
    windows: &'a [PersistedWindowState],
    requested_id: &str,
) -> Option<&'a str> {
    windows
        .iter()
        .find(|window| window.id == requested_id)
        .or_else(|| {
            windows
                .iter()
                .find(|window| window.id.ends_with(&format!("::{requested_id}")))
        })
        .map(|window| window.id.as_str())
}

fn status_label(status: WindowState) -> &'static str {
    match status {
        WindowState::Running => "running",
        WindowState::Starting => "starting",
        WindowState::Idle => "idle",
        WindowState::Waiting => "waiting",
        WindowState::Stopped => "stopped",
        WindowState::Error => "error",
    }
}

fn preset_label(preset: WindowPreset) -> &'static str {
    match preset {
        WindowPreset::Claude => "claude",
        WindowPreset::Codex => "codex",
        WindowPreset::Agent => "agent",
        _ => "unknown",
    }
}

fn config_error(message: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(message))
}

#[cfg(test)]
mod tests {
    use crate::persistence::WindowGeometry;
    use gwt_core::test_support::ScopedEnvVar;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn window(id: &str, preset: WindowPreset, agent_id: Option<&str>) -> PersistedWindowState {
        PersistedWindowState {
            id: id.to_string(),
            title: id.to_string(),
            purpose_title: None,
            dynamic_title: None,
            dynamic_title_detail: None,
            preset,
            geometry: WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            geometry_revision: 0,
            z_index: 1,
            status: WindowState::Running,
            placement: WindowPlacement::Canvas,
            persist: true,
            agent_id: agent_id.map(str::to_string),
            agent_color: None,
            lane_kind: WindowLaneKind::Unknown,
            tab_group_id: None,
            tab_group_active: false,
            session_id: None,
        }
    }

    #[test]
    fn parse_supports_agent_skill_modes() {
        assert_eq!(parse(&[]).unwrap(), PaneCommand::List);
        assert_eq!(parse(&[s("list")]).unwrap(), PaneCommand::List);
        assert_eq!(
            parse(&[s("agent-1")]).unwrap(),
            PaneCommand::Read {
                id: "agent-1".to_string(),
                lines: DEFAULT_READ_LINES,
            }
        );
        assert_eq!(
            parse(&[s("read"), s("agent-1"), s("--lines"), s("12")]).unwrap(),
            PaneCommand::Read {
                id: "agent-1".to_string(),
                lines: 12,
            }
        );
        assert_eq!(
            parse(&[s("stop"), s("agent-1")]).unwrap(),
            PaneCommand::Close {
                id: "agent-1".to_string(),
            }
        );
    }

    #[test]
    fn parse_supports_send_action_with_optional_pane_id() {
        assert_eq!(
            parse(&[s("send"), s("--text"), s("/goal tests pass")]).unwrap(),
            PaneCommand::Send {
                id: None,
                text: "/goal tests pass".to_string(),
            }
        );
        assert_eq!(
            parse(&[s("send"), s("agent-1"), s("--text"), s("/goal x")]).unwrap(),
            PaneCommand::Send {
                id: Some("agent-1".to_string()),
                text: "/goal x".to_string(),
            }
        );
        assert!(parse(&[s("send")]).is_err());
        assert!(parse(&[s("send"), s("agent-1")]).is_err());
        assert!(parse(&[s("send"), s("--text")]).is_err());
    }

    #[test]
    fn resolve_send_target_enforces_self_only_session_binding() {
        let mut own = window("tab-1::claude-1", WindowPreset::Claude, Some("claude"));
        own.session_id = Some("session-a".to_string());
        let mut other = window("tab-1::codex-1", WindowPreset::Codex, Some("codex"));
        other.session_id = Some("session-b".to_string());
        let windows = vec![own, other];

        // 対象省略 = 自 session の pane に解決される。
        assert_eq!(
            resolve_send_target(&windows, None, "session-a").unwrap(),
            "tab-1::claude-1"
        );
        // 明示指定も自 session の pane なら許可 (suffix 解決込み)。
        assert_eq!(
            resolve_send_target(&windows, Some("claude-1"), "session-a").unwrap(),
            "tab-1::claude-1"
        );
        // 他 session の pane 指定は self-only 違反として拒否 (SPEC-3050 AS3)。
        let denied = resolve_send_target(&windows, Some("codex-1"), "session-a").unwrap_err();
        assert!(denied.contains("not bound to this session"));
        // 未知の pane id。
        assert!(resolve_send_target(&windows, Some("ghost-1"), "session-a").is_err());
        // session に紐づく pane が無い場合。
        assert!(resolve_send_target(&windows, None, "session-zzz").is_err());
    }

    #[test]
    fn parse_pane_send_result_extracts_backend_reply() {
        let ok = serde_json::json!({
            "kind": "pane_send_result",
            "ok": true,
            "window_id": "tab-1::claude-1",
            "error": null
        });
        assert_eq!(
            parse_pane_send_result(&ok).unwrap(),
            Some(PaneSendReply {
                ok: true,
                window_id: Some("tab-1::claude-1".to_string()),
                error: None,
            })
        );

        let err = serde_json::json!({
            "kind": "pane_send_result",
            "ok": false,
            "window_id": null,
            "error": "no pane bound to session session-a"
        });
        assert_eq!(
            parse_pane_send_result(&err).unwrap(),
            Some(PaneSendReply {
                ok: false,
                window_id: None,
                error: Some("no pane bound to session session-a".to_string()),
            })
        );

        let unrelated = serde_json::json!({ "kind": "workspace_state" });
        assert_eq!(parse_pane_send_result(&unrelated).unwrap(), None);
    }

    #[test]
    fn ensure_trailing_submit_appends_carriage_return_once() {
        assert_eq!(ensure_trailing_submit("/goal x"), "/goal x\r");
        assert_eq!(ensure_trailing_submit("/goal x\r"), "/goal x\r");
        assert_eq!(ensure_trailing_submit("/goal x\n"), "/goal x\n");
    }

    #[test]
    fn pane_websocket_env_uses_dedicated_browser_listener() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _pane_url = ScopedEnvVar::set(GWT_PANE_WS_URL_ENV, "ws://127.0.0.1:46234/ws");
        let _hook_url = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            "http://127.0.0.1:45123/internal/hook-live",
        );

        assert_eq!(
            pane_websocket_url_from_env().expect("dedicated pane endpoint"),
            "ws://127.0.0.1:46234/ws"
        );
    }

    #[test]
    fn pane_websocket_env_never_guesses_a_pane_route_from_the_hook_endpoint() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _pane_url = ScopedEnvVar::unset(GWT_PANE_WS_URL_ENV);
        let _hook_url = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
            "http://127.0.0.1:61234/internal/hook-live",
        );

        let error = pane_websocket_url_from_env()
            .expect_err("the explicit pane WebSocket endpoint must remain required");

        assert!(error.contains(GWT_PANE_WS_URL_ENV), "{error}");
    }

    #[test]
    fn pane_websocket_request_carries_the_agent_capability_in_authorization() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "agent-capability-secret-sentinel",
        );

        let request = pane_websocket_request("ws://127.0.0.1:45123/internal/pane-ws")
            .expect("pane WebSocket request");

        assert_eq!(
            request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer agent-capability-secret-sentinel")
        );
        assert!(!request.uri().to_string().contains("secret-sentinel"));
    }

    #[test]
    fn pane_websocket_request_rejects_browser_listener_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "agent-capability-secret-sentinel",
        );

        let error = pane_websocket_request("ws://127.0.0.1:46234/ws")
            .expect_err("pane.* must never fall back to the browser listener");
        assert!(error.contains("/internal/pane-ws"), "{error}");
        assert!(pane_websocket_request("ws://127.0.0.1:46234/ws?token=forbidden").is_err());
        assert!(pane_websocket_request("ws://127.0.0.1:46234/internal/hook-live").is_err());
    }

    #[test]
    fn render_pane_list_filters_to_agent_terminal_windows() {
        let windows = vec![
            window("tab-1::shell-1", WindowPreset::Shell, None),
            window("tab-1::codex-1", WindowPreset::Codex, Some("codex")),
            window("tab-1::agent-1", WindowPreset::Agent, Some("custom")),
        ];

        let rendered = render_pane_list(&windows);

        assert!(!rendered.contains("shell-1"));
        assert!(rendered.contains("tab-1::codex-1\trunning\tcodex"));
        assert!(rendered.contains("tab-1::agent-1\trunning\tcustom"));
    }

    #[test]
    fn render_pane_list_labels_pre_lifecycle_agents_starting() {
        let mut windows = vec![window("tab-1::codex-1", WindowPreset::Codex, Some("codex"))];
        windows[0].status = WindowState::Starting;

        let rendered = render_pane_list(&windows);

        assert!(rendered.contains("tab-1::codex-1\tstarting\tcodex"));
    }

    #[test]
    fn workspace_windows_are_scoped_to_project_root() {
        let value = serde_json::json!({
            "kind": "workspace_state",
            "workspace": {
                "tabs": [
                    {
                        "project_root": "/repo/one",
                        "workspace": {
                            "windows": [window("one::agent-1", WindowPreset::Agent, Some("one"))],
                        },
                    },
                    {
                        "project_root": "/repo/two",
                        "workspace": {
                            "windows": [window("two::agent-1", WindowPreset::Agent, Some("two"))],
                        },
                    },
                ],
            },
        });

        let windows = parse_workspace_windows(&value, "/repo/two").unwrap();

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "two::agent-1");
    }

    #[test]
    fn workspace_windows_keep_empty_matched_project_scoped() {
        let value = serde_json::json!({
            "kind": "workspace_state",
            "workspace": {
                "tabs": [
                    {
                        "project_root": "/repo/empty",
                        "workspace": { "windows": [] },
                    },
                    {
                        "project_root": "/repo/other",
                        "workspace": {
                            "windows": [window("other::agent-1", WindowPreset::Agent, Some("other"))],
                        },
                    },
                ],
            },
        });

        let windows = parse_workspace_windows(&value, "/repo/empty").unwrap();

        assert!(windows.is_empty());
    }

    #[test]
    fn render_snapshot_lines_keeps_requested_tail() {
        assert_eq!(render_snapshot_lines("a\nb\nc\n", 2), "b\nc\n");
    }

    fn workspace_state_for_test(project_root: &str, windows: Vec<PersistedWindowState>) -> Value {
        json!({
            "kind": "workspace_state",
            "workspace": {
                "tabs": [{
                    "project_root": project_root,
                    "workspace": { "windows": windows },
                }],
            },
        })
    }

    async fn next_frontend_json(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> Value {
        let message = socket
            .next()
            .await
            .expect("frontend frame")
            .expect("valid frontend frame");
        let Message::Text(text) = message else {
            panic!("frontend frame must be text");
        };
        serde_json::from_str(text.as_ref()).expect("frontend frame JSON")
    }

    async fn next_frontend_kind(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    ) -> String {
        let value = next_frontend_json(socket).await;
        value["kind"]
            .as_str()
            .expect("frontend frame kind")
            .to_string()
    }

    #[derive(Clone, Copy)]
    enum SelfCloseMockReply {
        Matching,
        Mismatched,
        UnrelatedThenMatching,
        CloseWithoutReply,
    }

    async fn spawn_close_pane_mock(
        project_root: &'static str,
        target: PersistedWindowState,
        post_close_windows: Option<Vec<PersistedWindowState>>,
        self_close_reply: SelfCloseMockReply,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let initial_state = workspace_state_for_test(project_root, vec![target]);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind pane mock");
        let address = listener.local_addr().expect("pane mock address");
        let server = tokio::spawn(async move {
            let mut received_kinds = Vec::new();
            for connection_index in 0..2 {
                let (stream, _) = listener.accept().await.expect("accept pane connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("accept pane websocket");

                received_kinds.push(next_frontend_kind(&mut socket).await);
                socket
                    .send(Message::Text(initial_state.to_string().into()))
                    .await
                    .expect("send workspace state");

                if connection_index == 1 {
                    let close = next_frontend_json(&mut socket).await;
                    received_kinds.push(
                        close["kind"]
                            .as_str()
                            .expect("close frontend kind")
                            .to_string(),
                    );
                    if let Some(windows) = post_close_windows.as_ref() {
                        received_kinds.push(
                            tokio::time::timeout(
                                Duration::from_secs(1),
                                next_frontend_kind(&mut socket),
                            )
                            .await
                            .expect("post-close frontend_ready timeout"),
                        );
                        let post_close_state =
                            workspace_state_for_test(project_root, windows.clone());
                        socket
                            .send(Message::Text(post_close_state.to_string().into()))
                            .await
                            .expect("send post-close workspace state");
                    } else {
                        let request_id = close["request_id"]
                            .as_str()
                            .expect("self-close request correlation");
                        match self_close_reply {
                            SelfCloseMockReply::Matching
                            | SelfCloseMockReply::Mismatched
                            | SelfCloseMockReply::UnrelatedThenMatching => {
                                let response_request_id = match self_close_reply {
                                    SelfCloseMockReply::Matching
                                    | SelfCloseMockReply::UnrelatedThenMatching => request_id,
                                    SelfCloseMockReply::Mismatched => "wrong-request-id",
                                    SelfCloseMockReply::CloseWithoutReply => unreachable!(),
                                };
                                if matches!(
                                    self_close_reply,
                                    SelfCloseMockReply::UnrelatedThenMatching
                                ) {
                                    socket
                                        .send(Message::Text(
                                            json!({ "kind": "runtime_hook_event" })
                                                .to_string()
                                                .into(),
                                        ))
                                        .await
                                        .expect("send unrelated backend frame");
                                }
                                let accepted = json!({
                                    "kind": "pane_close_accepted",
                                    "request_id": response_request_id,
                                    "window_id": close["id"],
                                });
                                socket
                                    .send(Message::Text(accepted.to_string().into()))
                                    .await
                                    .expect("send self-close acceptance");
                            }
                            SelfCloseMockReply::CloseWithoutReply => {
                                socket.close(None).await.expect("close without acceptance");
                            }
                        }
                    }
                }
            }
            received_kinds
        });

        (format!("ws://{address}/internal/pane-ws"), server)
    }

    #[test]
    fn self_pane_close_requires_matching_server_acceptance() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "self-close-capability",
        );
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-self");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let project_root = "/repo/self";
            let mut own = window("tab-self::agent-self", WindowPreset::Codex, Some("codex"));
            own.session_id = Some("session-self".to_string());
            let (ws_url, server) =
                spawn_close_pane_mock(project_root, own, None, SelfCloseMockReply::Matching).await;

            let result = close_pane(&ws_url, project_root, "agent-self").await;
            let received_kinds = server.await.expect("pane mock task");

            assert_eq!(result, Ok("close requested agent-self\n".to_string()));
            assert_eq!(
                received_kinds,
                vec!["frontend_ready", "frontend_ready", "close_window"],
                "self-close must not send a second frontend_ready after revocation"
            );
        });
    }

    #[test]
    fn self_pane_close_ignores_unrelated_frames_before_matching_acceptance() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "self-close-capability",
        );
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-self");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let project_root = "/repo/self";
            let mut own = window("tab-self::agent-self", WindowPreset::Codex, Some("codex"));
            own.session_id = Some("session-self".to_string());
            let (ws_url, server) = spawn_close_pane_mock(
                project_root,
                own,
                None,
                SelfCloseMockReply::UnrelatedThenMatching,
            )
            .await;

            let result = close_pane(&ws_url, project_root, "agent-self").await;
            server.await.expect("pane mock task");

            assert_eq!(result, Ok("close requested agent-self\n".to_string()));
        });
    }

    #[test]
    fn self_pane_close_absolute_deadline_is_not_extended_by_unrelated_frames() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind flood mock");
            let address = listener.local_addr().expect("flood mock address");
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.expect("accept flood connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("accept flood websocket");
                loop {
                    if socket
                        .send(Message::Text(
                            json!({ "kind": "runtime_hook_event" }).to_string().into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            });
            let (mut socket, _) = connect_async(format!("ws://{address}/internal/pane-ws"))
                .await
                .expect("connect flood websocket");
            let started = std::time::Instant::now();

            let error = wait_for_pane_close_acceptance(
                &mut socket,
                "expected-request",
                "tab-self::agent-self",
                Duration::from_millis(50),
            )
            .await
            .expect_err("unrelated frames must not extend the absolute deadline");

            assert!(error.contains("timed out"), "{error}");
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "absolute deadline was extended by unrelated traffic: {:?}",
                started.elapsed()
            );
            drop(socket);
            server.abort();
            let _ = server.await;
        });
    }

    #[test]
    fn own_pane_close_never_reports_success_without_matching_ambient_session() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "self-close-capability",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            for ambient_session in [None, Some("different-session")] {
                let _session = match ambient_session {
                    Some(session_id) => ScopedEnvVar::set(GWT_SESSION_ID_ENV, session_id),
                    None => ScopedEnvVar::unset(GWT_SESSION_ID_ENV),
                };
                let project_root = "/repo/self";
                let mut own = window("tab-self::agent-self", WindowPreset::Codex, Some("codex"));
                own.session_id = Some("session-self".to_string());
                let (ws_url, server) = spawn_close_pane_mock(
                    project_root,
                    own.clone(),
                    Some(vec![own]),
                    SelfCloseMockReply::Matching,
                )
                .await;

                let error = close_pane(&ws_url, project_root, "agent-self")
                    .await
                    .expect_err("rejected uncorrelated self-close must not report success");
                let received_kinds = server.await.expect("pane mock task");

                assert!(
                    error.contains("requires a correlated acceptance"),
                    "{error}"
                );
                assert_eq!(
                    received_kinds,
                    vec![
                        "frontend_ready",
                        "frontend_ready",
                        "close_window",
                        "frontend_ready"
                    ]
                );
            }
        });
    }

    #[test]
    fn self_pane_close_rejects_disconnect_without_acceptance() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "self-close-capability",
        );
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-self");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let project_root = "/repo/self";
            let mut own = window("tab-self::agent-self", WindowPreset::Codex, Some("codex"));
            own.session_id = Some("session-self".to_string());
            let (ws_url, server) = spawn_close_pane_mock(
                project_root,
                own,
                None,
                SelfCloseMockReply::CloseWithoutReply,
            )
            .await;

            let error = close_pane(&ws_url, project_root, "agent-self")
                .await
                .expect_err("disconnect before acceptance must fail");
            let received_kinds = server.await.expect("pane mock task");

            assert!(error.starts_with("pane "), "{error}");
            assert_eq!(
                received_kinds,
                vec!["frontend_ready", "frontend_ready", "close_window"]
            );
        });
    }

    #[test]
    fn self_pane_close_rejects_mismatched_acceptance() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "self-close-capability",
        );
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-self");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let project_root = "/repo/self";
            let mut own = window("tab-self::agent-self", WindowPreset::Codex, Some("codex"));
            own.session_id = Some("session-self".to_string());
            let (ws_url, server) =
                spawn_close_pane_mock(project_root, own, None, SelfCloseMockReply::Mismatched)
                    .await;

            let error = close_pane(&ws_url, project_root, "agent-self")
                .await
                .expect_err("mismatched acceptance must fail");
            server.await.expect("pane mock task");

            assert!(error.starts_with("pane "), "{error}");
        });
    }

    #[test]
    fn non_self_pane_close_keeps_authoritative_post_close_readback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "peer-close-capability",
        );
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-self");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let project_root = "/repo/peer";
            let mut peer = window("tab-peer::agent-peer", WindowPreset::Codex, Some("codex"));
            peer.session_id = Some("session-peer".to_string());
            let (ws_url, server) = spawn_close_pane_mock(
                project_root,
                peer,
                Some(Vec::new()),
                SelfCloseMockReply::Matching,
            )
            .await;

            let result = close_pane(&ws_url, project_root, "agent-peer").await;
            let received_kinds = server.await.expect("pane mock task");

            assert_eq!(result, Ok("closed agent-peer\n".to_string()));
            assert_eq!(
                received_kinds,
                vec![
                    "frontend_ready",
                    "frontend_ready",
                    "close_window",
                    "frontend_ready"
                ],
                "non-self close must retain authoritative post-close readback"
            );
        });
    }
}
