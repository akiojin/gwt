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
        protocol::frame::coding::CloseCode,
        Error as WebSocketError, Message,
    },
};

use crate::{
    persistence::{PersistedWindowState, WindowState},
    preset::WindowPreset,
};

#[cfg(test)]
use crate::persistence::WindowPlacement;
#[cfg(test)]
use crate::persistence::WindowWorktreeForm;

use super::{CliEnv, CliParseError, PaneCommand};

const DEFAULT_READ_LINES: usize = 50;
const PROJECT_ROOT_ENV: &str = "GWT_PROJECT_ROOT";
/// How long one pane request waits for the GUI to answer.
///
/// Every `pane.*` reply is produced on the GUI's single event loop, so the
/// wait is dominated by whatever that loop is doing rather than by the size
/// of the reply: measured round trips are ~5ms while the loop is idle, and
/// they run past seconds while it drives a Work/branch scan that spawns tens
/// of `git` processes per second. A two-second budget therefore turned an
/// ordinary stall into a hard failure (#3510). The budget covers the stall
/// instead, and the idle case is unaffected because it never waits.
const BACKEND_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const PM_MESSAGE_RESULT_DEADLINE: Duration = Duration::from_secs(7);
const PM_MESSAGE_SEND_DEADLINE: Duration = Duration::from_secs(18);
const ISSUE_MONITOR_SCAN_RESULT_DEADLINE: Duration = Duration::from_secs(5);
const PM_TARGET_REFUSAL: &str =
    "pm.message.send refused: target is not an authorized live agent pane";

type PaneWebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// SPEC-3431 FR-124: ask the authenticated GUI authority for one project
/// Monitor scan. The request intentionally carries no project path; the
/// agent WebSocket principal is the sole scope authority.
#[cfg_attr(unix, allow(dead_code))]
pub(super) fn request_issue_monitor_scan_now(project_root: &Path) -> Result<(), String> {
    let ws_url = pane_websocket_url_from_env().map_err(|error| {
        tracing::warn!(%error, "Issue Monitor GUI command endpoint is unavailable");
        "gui_command_unavailable".to_string()
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            tracing::warn!(%error, "failed to create Issue Monitor GUI command runtime");
            "gui_command_unavailable".to_string()
        })?;
    let expected_project_scope = gwt_core::paths::project_scope_hash(project_root);
    runtime.block_on(request_issue_monitor_scan_now_async(
        &ws_url,
        expected_project_scope.as_str(),
    ))
}

async fn request_issue_monitor_scan_now_async(
    ws_url: &str,
    expected_project_scope: &str,
) -> Result<(), String> {
    let request = pane_websocket_request(ws_url).map_err(|error| {
        tracing::warn!(%error, "Issue Monitor GUI command request is unavailable");
        "gui_command_unavailable".to_string()
    })?;
    let mut socket = connect_async(request)
        .await
        .map(|(socket, _)| socket)
        .map_err(|error| {
            let code = issue_monitor_scan_connect_error(&error);
            tracing::warn!(%error, code, "Issue Monitor GUI command connection failed");
            code.to_string()
        })?;
    send_frontend_event(
        &mut socket,
        json!({
            "kind": "agent_issue_monitor_scan_now",
            "expected_project_scope": expected_project_scope,
        }),
    )
    .await
    .map_err(|error| {
        tracing::warn!(%error, "Issue Monitor GUI scan request failed");
        "scan_delivery_unknown".to_string()
    })?;
    tokio::time::timeout(ISSUE_MONITOR_SCAN_RESULT_DEADLINE, async {
        loop {
            let value = next_issue_monitor_scan_json(&mut socket).await?;
            let Some(reply) = parse_issue_monitor_scan_result(&value).map_err(|error| {
                tracing::warn!(%error, "Issue Monitor GUI scan response was invalid");
                "scan_delivery_unknown".to_string()
            })?
            else {
                continue;
            };
            return if reply.accepted {
                Ok(())
            } else {
                Err(reply
                    .reason
                    .unwrap_or_else(|| "scan_request_rejected".to_string()))
            };
        }
    })
    .await
    .map_err(|_| "scan_delivery_unknown".to_string())?
}

fn issue_monitor_scan_connect_error(error: &WebSocketError) -> &'static str {
    match error {
        WebSocketError::Http(response) if matches!(response.status().as_u16(), 401 | 403 | 409) => {
            "gui_capability_rejected"
        }
        WebSocketError::Http(response) if response.status().as_u16() == 503 => {
            "gui_execution_authority_unavailable"
        }
        _ => "gui_command_unavailable",
    }
}

async fn next_issue_monitor_scan_json(socket: &mut PaneWebSocket) -> Result<Value, String> {
    let message = socket
        .next()
        .await
        .ok_or_else(|| "scan_delivery_unknown".to_string())?
        .map_err(|error| {
            tracing::warn!(%error, "Issue Monitor GUI scan response failed");
            "scan_delivery_unknown".to_string()
        })?;
    match message {
        Message::Text(text) => serde_json::from_str(text.as_ref()).map_err(|error| {
            tracing::warn!(%error, "Issue Monitor GUI scan response was invalid JSON");
            "scan_delivery_unknown".to_string()
        }),
        Message::Binary(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            tracing::warn!(%error, "Issue Monitor GUI scan response was invalid JSON");
            "scan_delivery_unknown".to_string()
        }),
        Message::Close(Some(frame)) if frame.code == CloseCode::Policy => {
            Err("gui_capability_rejected".to_string())
        }
        Message::Close(_) => Err("scan_delivery_unknown".to_string()),
        _ => Err("scan_delivery_unknown".to_string()),
    }
}

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
        PaneCommand::PmSend {
            project_root: explicit_project_root,
            id,
            text,
        } => send_pm_pane_input(ws_url, explicit_project_root.as_deref(), &id, &text).await,
    }
}

/// SPEC-3431 FR-111 (T-206): PM-privileged delivery into another agent pane
/// of the same project. Not a loosened `pane.send`: the SPEC-3050 self-only
/// contract stays intact for every ordinary agent, and this path exists only
/// for the live registered PM. The authenticated WebSocket principal is the
/// sole caller identity; all registration and target authority checks remain
/// server-side and the terminal result returns on this same connection.
async fn send_pm_pane_input(
    ws_url: &str,
    project_root: Option<&str>,
    requested_id: &str,
    text: &str,
) -> Result<String, String> {
    let operation_id = uuid::Uuid::new_v4().hyphenated().to_string();
    tokio::time::timeout(PM_MESSAGE_SEND_DEADLINE, async {
        let mut expected_window_id = None::<String>;
        let mut response_loss = None::<String>;
        for attempt in 0..2 {
            let mut socket = connect_pane_websocket(ws_url).await?;
            send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;
            let windows = next_pm_workspace_windows(&mut socket, project_root).await?;
            let window_id = if let Some(expected) = expected_window_id.as_deref() {
                expected.to_string()
            } else {
                resolve_pm_send_target(&windows, requested_id)?.id.clone()
            };
            if expected_window_id
                .as_deref()
                .is_some_and(|expected| expected != window_id)
            {
                return Err(
                    "pm.message.send target changed before same-operation replay".to_string(),
                );
            }
            expected_window_id.get_or_insert_with(|| window_id.clone());
            if let Err(error) = send_frontend_event(
                &mut socket,
                json!({
                    "kind": "pm_pane_send_input",
                    "operation_id": operation_id,
                    "window_id": window_id,
                    "text": ensure_trailing_submit(text),
                }),
            )
            .await
            {
                if attempt == 0 {
                    response_loss = Some(error);
                    continue;
                }
                return Err(format!(
                    "pm.message.send request transmission was lost after same-operation replay: {error}"
                ));
            }

            let reply = match wait_for_pm_message_send_result(
                &mut socket,
                &operation_id,
                &window_id,
                PM_MESSAGE_RESULT_DEADLINE,
            )
            .await
            {
                Ok(reply) => Some(reply),
                Err(error) if attempt == 0 => {
                    response_loss = Some(error);
                    None
                }
                Err(error) => {
                    return Err(format!(
                        "pm.message.send response was lost after same-operation replay: {error}"
                    ))
                }
            };
            let Some(reply) = reply else {
                continue;
            };
            return match reply.status.as_str() {
                "delivered" => Ok(format!("pm message delivered to {window_id}\n")),
                "queued" => Ok(format!("pm message queued for {window_id}\n")),
                "failed" => Err(format!(
                    "pm message failed: {}",
                    reply.reason.unwrap_or_else(|| "unknown reason".to_string())
                )),
                status => Err(format!(
                    "pm_message_send_result has invalid status {status}"
                )),
            };
        }
        Err(format!(
            "pm.message.send response was lost: {}",
            response_loss.unwrap_or_else(|| "unknown transport error".to_string())
        ))
    })
    .await
    .map_err(|_| {
        format!(
            "pm.message.send timed out after {} seconds",
            PM_MESSAGE_SEND_DEADLINE.as_secs()
        )
    })?
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
    request_window_list_with_timeout(ws_url, project_root, BACKEND_RESPONSE_TIMEOUT).await
}

async fn request_window_list_with_timeout(
    ws_url: &str,
    project_root: &str,
    response_timeout: Duration,
) -> Result<Vec<PersistedWindowState>, String> {
    let mut socket = connect_pane_websocket(ws_url).await?;
    send_frontend_event(&mut socket, json!({ "kind": "list_windows" })).await?;

    let listed = next_workspace_windows_with_timeout(
        &mut socket,
        project_root,
        "pane list",
        response_timeout,
    )
    .await;
    if listed.is_ok() {
        return listed;
    }

    // `gwtd` runs from the installed bundle while the GUI keeps running the
    // build it was started with, so an in-place upgrade leaves the two on
    // different pane protocols until the app restarts. A backend without the
    // lightweight route drops `list_windows` without replying; the full sync
    // request is understood by every version, so falling back to it keeps
    // `pane.list` answering across that window.
    //
    // The fallback waits out the whole response budget first, which is
    // deliberate: a merely stalled backend answers the light route within the
    // budget, so only a backend that truly ignores the request pays the
    // second round and the heavier reply it triggers.
    send_frontend_event(&mut socket, json!({ "kind": "frontend_ready" })).await?;
    next_workspace_windows_with_timeout(&mut socket, project_root, "pane list", response_timeout)
        .await
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
    next_backend_json_with_timeout(socket, BACKEND_RESPONSE_TIMEOUT).await
}

async fn next_backend_json_with_timeout(
    socket: &mut PaneWebSocket,
    response_timeout: Duration,
) -> Result<Value, String> {
    tokio::time::timeout(response_timeout, next_backend_json_unbounded(socket))
        .await
        .map_err(|_| "pane websocket timed out waiting for backend response".to_string())?
}

async fn next_backend_json_unbounded(socket: &mut PaneWebSocket) -> Result<Value, String> {
    let message = socket
        .next()
        .await
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

async fn wait_for_pm_message_send_result(
    socket: &mut PaneWebSocket,
    expected_operation_id: &str,
    expected_window_id: &str,
    deadline_after: Duration,
) -> Result<PmMessageSendReply, String> {
    tokio::time::timeout(deadline_after, async {
        loop {
            let value = next_backend_json_unbounded(socket).await?;
            if let Some(reply) =
                parse_pm_message_send_result(&value, expected_operation_id, expected_window_id)?
            {
                return Ok::<PmMessageSendReply, String>(reply);
            }
        }
    })
    .await
    .map_err(|_| "pm.message.send timed out waiting for its terminal result".to_string())?
}

async fn next_workspace_windows(
    socket: &mut PaneWebSocket,
    project_root: &str,
    context: &str,
) -> Result<Vec<PersistedWindowState>, String> {
    next_workspace_windows_with_timeout(socket, project_root, context, BACKEND_RESPONSE_TIMEOUT)
        .await
}

async fn next_workspace_windows_with_timeout(
    socket: &mut PaneWebSocket,
    project_root: &str,
    context: &str,
    response_timeout: Duration,
) -> Result<Vec<PersistedWindowState>, String> {
    for _ in 0..32 {
        let value = next_backend_json_with_timeout(socket, response_timeout)
            .await
            .map_err(|error| format!("{context}: {error}"))?;
        if let Some(windows) = parse_workspace_windows(&value, project_root) {
            return Ok(windows);
        }
    }
    Err(format!("{context}: backend did not return workspace_state"))
}

/// Read the project projection from the same authenticated connection used
/// for the PM mutation. When no explicit root is supplied, the server-side
/// capability projection is authoritative and no ambient cwd/env path is
/// consulted.
async fn next_pm_workspace_windows(
    socket: &mut PaneWebSocket,
    project_root: Option<&str>,
) -> Result<Vec<PersistedWindowState>, String> {
    for _ in 0..32 {
        let value = next_backend_json(socket).await?;
        if value.get("kind").and_then(Value::as_str) != Some("workspace_state") {
            continue;
        }
        let tabs = value
            .get("workspace")
            .and_then(|workspace| workspace.get("tabs"))
            .and_then(Value::as_array)
            .ok_or_else(|| "pm.message.send: workspace_state missing tabs".to_string())?;
        let scope_key = |root: &str| {
            gwt_core::paths::project_scope_hash(Path::new(root))
                .as_str()
                .to_string()
        };
        let mut matched = project_root.is_none();
        let mut windows = Vec::new();
        for tab in tabs {
            let tab_root = tab.get("project_root").and_then(Value::as_str);
            let authorized = match (project_root, tab_root) {
                (None, _) => true,
                (Some(requested), Some(root)) => tab_owns_caller(root, requested, &scope_key),
                (Some(_), None) => false,
            };
            if !authorized {
                continue;
            }
            matched = true;
            let Some(tab_windows) = tab
                .get("workspace")
                .and_then(|workspace| workspace.get("windows"))
            else {
                continue;
            };
            let mut parsed = serde_json::from_value::<Vec<PersistedWindowState>>(
                tab_windows.clone(),
            )
            .map_err(|error| format!("pm.message.send: invalid workspace projection: {error}"))?;
            windows.append(&mut parsed);
        }
        if !matched {
            return Err("pm.message.send failed: explicit project_root is outside the authenticated project scope".to_string());
        }
        return Ok(windows);
    }
    Err("pm.message.send: backend did not return workspace_state".to_string())
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

#[derive(Debug, PartialEq, Eq)]
struct PmMessageSendReply {
    status: String,
    reason: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct IssueMonitorScanReply {
    accepted: bool,
    reason: Option<String>,
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

fn parse_pm_message_send_result(
    value: &Value,
    expected_operation_id: &str,
    expected_window_id: &str,
) -> Result<Option<PmMessageSendReply>, String> {
    if value.get("kind").and_then(Value::as_str) != Some("pm_message_send_result") {
        return Ok(None);
    }
    if value.get("operation_id").and_then(Value::as_str) != Some(expected_operation_id)
        || value.get("window_id").and_then(Value::as_str) != Some(expected_window_id)
    {
        return Ok(None);
    }
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "pm_message_send_result missing status".to_string())?
        .to_string();
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(Some(PmMessageSendReply { status, reason }))
}

fn parse_issue_monitor_scan_result(value: &Value) -> Result<Option<IssueMonitorScanReply>, String> {
    if value.get("kind").and_then(Value::as_str) != Some("issue_monitor_scan_request_result") {
        return Ok(None);
    }
    let accepted = value
        .get("accepted")
        .and_then(Value::as_bool)
        .ok_or_else(|| "issue_monitor_scan_request_result missing accepted".to_string())?;
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(Some(IssueMonitorScanReply { accepted, reason }))
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
    parse_workspace_windows_scoped(value, project_root, |root| {
        gwt_core::paths::project_scope_hash(Path::new(root))
            .as_str()
            .to_string()
    })
}

/// Whether `tab_root` is the project that owns the caller sitting at
/// `caller_root`.
///
/// Every launch sets `GWT_PROJECT_ROOT` to the agent's working dir, so a
/// caller's root is always a worktree and never equals a tab's project root.
/// Path equality alone therefore never matched for anyone, and the no-match
/// fallback in [`parse_workspace_windows_scoped`] handed back every window
/// from every open project.
///
/// Ownership has three shapes because gwt materializes worktrees in three
/// places: inside the project container (`work/`, `.intake*`), outside it
/// under `~/.gwt/projects/` (the resident PM), and — when the project root is
/// itself a git repo — anywhere that shares its repo identity.
fn tab_owns_caller(tab_root: &str, caller_root: &str, scope_key: &impl Fn(&str) -> String) -> bool {
    if tab_root == caller_root {
        return true;
    }
    let tab_path = Path::new(tab_root);
    if Path::new(caller_root).starts_with(tab_path) {
        return true;
    }
    if crate::pm_registry::pm_worktree_path_for_repo_path(tab_path) == Path::new(caller_root) {
        return true;
    }
    scope_key(tab_root) == scope_key(caller_root)
}

/// Select the windows the caller is allowed to see, scoped to the project that
/// owns it. `scope_key` maps a path to its repo identity; it is injected so the
/// selection stays testable without real repositories.
fn parse_workspace_windows_scoped(
    value: &Value,
    project_root: &str,
    scope_key: impl Fn(&str) -> String,
) -> Option<Vec<PersistedWindowState>> {
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
            let owns_caller = tab
                .get("project_root")
                .and_then(Value::as_str)
                .is_some_and(|root| tab_owns_caller(root, project_root, &scope_key));
            if owns_caller {
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

fn resolve_pm_send_target<'a>(
    windows: &'a [PersistedWindowState],
    requested_id: &str,
) -> Result<&'a PersistedWindowState, String> {
    let window = if let Some(exact) = windows.iter().find(|window| window.id == requested_id) {
        exact
    } else {
        let mut suffix_matches = windows
            .iter()
            .filter(|window| window.id.ends_with(&format!("::{requested_id}")));
        let Some(candidate) = suffix_matches.next() else {
            return Err(PM_TARGET_REFUSAL.to_string());
        };
        if suffix_matches.next().is_some() {
            return Err(PM_TARGET_REFUSAL.to_string());
        }
        candidate
    };
    let supported_preset = pm_window_has_exact_prompt_ack(window);
    let live_status = matches!(
        window.status,
        WindowState::Running | WindowState::Idle | WindowState::Waiting
    );
    let valid_session = window.session_id.as_deref().is_some_and(|session_id| {
        gwt_agent::validate_session_id_path_component(session_id).is_ok()
    });
    if !supported_preset || !live_status || !valid_session {
        return Err(PM_TARGET_REFUSAL.to_string());
    }
    Ok(window)
}

fn pm_window_has_exact_prompt_ack(window: &PersistedWindowState) -> bool {
    matches!(window.preset, WindowPreset::Claude | WindowPreset::Codex)
        || (window.preset == WindowPreset::Agent
            && matches!(window.agent_id.as_deref(), Some("claude" | "codex")))
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
            worktree_form: WindowWorktreeForm::Unknown,
            tab_group_id: None,
            tab_group_active: false,
            session_id: None,
            is_pm: false,
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
    fn parse_issue_monitor_scan_result_distinguishes_acceptance_from_unavailable() {
        let accepted = serde_json::json!({
            "kind": "issue_monitor_scan_request_result",
            "accepted": true,
            "reason": null,
        });
        assert_eq!(
            parse_issue_monitor_scan_result(&accepted).unwrap(),
            Some(IssueMonitorScanReply {
                accepted: true,
                reason: None,
            })
        );

        let unavailable = serde_json::json!({
            "kind": "issue_monitor_scan_request_result",
            "accepted": false,
            "reason": "monitor_disabled",
        });
        assert_eq!(
            parse_issue_monitor_scan_result(&unavailable).unwrap(),
            Some(IssueMonitorScanReply {
                accepted: false,
                reason: Some("monitor_disabled".to_string()),
            })
        );
        assert_eq!(
            parse_issue_monitor_scan_result(&serde_json::json!({"kind": "workspace_state"}))
                .unwrap(),
            None
        );
    }

    #[test]
    fn issue_monitor_scan_connect_error_preserves_capability_and_authority_diagnostics() {
        let http_error = |status| {
            WebSocketError::Http(Box::new(
                tokio_tungstenite::tungstenite::http::Response::builder()
                    .status(status)
                    .body(None)
                    .expect("HTTP response"),
            ))
        };
        assert_eq!(
            issue_monitor_scan_connect_error(&http_error(401)),
            "gui_capability_rejected"
        );
        assert_eq!(
            issue_monitor_scan_connect_error(&http_error(409)),
            "gui_capability_rejected"
        );
        assert_eq!(
            issue_monitor_scan_connect_error(&http_error(503)),
            "gui_execution_authority_unavailable"
        );
        assert_eq!(
            issue_monitor_scan_connect_error(&WebSocketError::ConnectionClosed),
            "gui_command_unavailable"
        );
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
    fn render_pane_list_projects_approval_wait_as_waiting() {
        let mut windows = vec![window("tab-1::agent-1", WindowPreset::Agent, Some("codex"))];
        windows[0].status = WindowState::Waiting;

        let rendered = render_pane_list(&windows);

        assert!(rendered.contains("tab-1::agent-1\twaiting\tcodex"));
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

    /// Every agent's `GWT_PROJECT_ROOT` is its own worktree, never the tab's
    /// project root, so exact-path matching never matched for anyone and the
    /// no-match fallback handed back every window from every open project.
    /// Verified live: this session's `GWT_PROJECT_ROOT` is a worktree that
    /// matches none of the three open tabs.
    ///
    /// Scoping by project ownership is what makes `pane.list` / `pane.read`
    /// safe to hand to an agent — a worktree resolves to the project that
    /// owns it, and unrelated projects stay invisible.
    #[test]
    fn workspace_windows_from_a_worktree_resolve_to_the_owning_project() {
        // Distinct identities: ownership must come from the path shapes below,
        // never from an accidental hash collision.
        let scope_key = |root: &str| format!("hash-of-{root}");
        let value = two_project_workspace_state();

        // gwt materializes work/intake worktrees inside the project container.
        let windows =
            parse_workspace_windows_scoped(&value, "/repo/two/work/issue-1", scope_key).unwrap();
        assert_eq!(windows.len(), 1, "only the owning project's windows");
        assert_eq!(windows[0].id, "two::agent-1");

        // The resident PM's worktree lives outside the checkout entirely.
        let pm_worktree =
            crate::pm_registry::pm_worktree_path_for_repo_path(Path::new("/repo/two"));
        let windows =
            parse_workspace_windows_scoped(&value, &pm_worktree.to_string_lossy(), scope_key)
                .unwrap();
        assert_eq!(windows.len(), 1, "the PM sees only its own project");
        assert_eq!(windows[0].id, "two::agent-1");
    }

    /// When the project root is itself a git repo, a worktree elsewhere on
    /// disk is still the same project.
    #[test]
    fn workspace_windows_match_a_worktree_sharing_the_project_repo_identity() {
        let scope_key = |root: &str| match root {
            "/repo/two" | "/elsewhere/wt" => "same-repo".to_string(),
            other => format!("hash-of-{other}"),
        };

        let windows = parse_workspace_windows_scoped(
            &two_project_workspace_state(),
            "/elsewhere/wt",
            scope_key,
        )
        .unwrap();

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "two::agent-1");
    }

    fn two_project_workspace_state() -> Value {
        json!({
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
        })
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

    async fn spawn_window_list_mock(
        project_root: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind pane list mock");
        let address = listener.local_addr().expect("pane list mock address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener
                .accept()
                .await
                .expect("accept pane list connection");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept pane list websocket");
            let request_kind = next_frontend_kind(&mut socket).await;
            let state = workspace_state_for_test(
                project_root,
                vec![window(
                    "tab-project::agent-project",
                    WindowPreset::Agent,
                    Some("codex"),
                )],
            );
            socket
                .send(Message::Text(state.to_string().into()))
                .await
                .expect("send pane list workspace state");
            request_kind
        });

        (format!("ws://{address}/internal/pane-ws"), server)
    }

    #[test]
    fn request_window_list_uses_lightweight_list_windows_request() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "pane-list-capability",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane list test runtime");

        runtime.block_on(async {
            let project_root = "/repo/project";
            let (ws_url, server) = spawn_window_list_mock(project_root).await;

            let windows = request_window_list(&ws_url, project_root)
                .await
                .expect("pane list response");
            let request_kind = server.await.expect("pane list mock task");

            assert_eq!(request_kind, "list_windows");
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0].id, "tab-project::agent-project");
        });
    }

    #[test]
    fn request_window_list_identifies_backend_response_timeout() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "pane-list-capability",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane list test runtime");

        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind pane list mock");
            let address = listener.local_addr().expect("pane list mock address");
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (stream, _) = listener
                    .accept()
                    .await
                    .expect("accept pane list connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("accept pane list websocket");
                assert_eq!(next_frontend_kind(&mut socket).await, "list_windows");
                let _socket = socket;
                let _ = release_rx.await;
            });

            let error = request_window_list_with_timeout(
                &format!("ws://{address}/internal/pane-ws"),
                "/repo/project",
                Duration::from_millis(20),
            )
            .await
            .expect_err("pane list response must time out");
            release_tx.send(()).expect("release pane list mock");
            server.await.expect("pane list mock task");

            assert_eq!(
                error,
                "pane list: pane websocket timed out waiting for backend response"
            );
        });
    }

    /// Issue #3510: the GUI answers pane requests from its single event loop,
    /// which stalls for seconds at a time while it drives long synchronous
    /// work (a Work/branch scan spawns tens of `git` processes per second).
    /// Measured replies are ~5ms when the loop is idle, so a client budget
    /// that gives up after two seconds turns an ordinary stall into a hard
    /// `pane.list` failure. The budget must outlast a multi-second stall.
    #[test]
    fn request_window_list_outlasts_a_multi_second_backend_stall() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "pane-list-capability",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane list test runtime");

        runtime.block_on(async {
            let project_root = "/repo/project";
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind pane list mock");
            let address = listener.local_addr().expect("pane list mock address");
            let server = tokio::spawn(async move {
                let (stream, _) = listener
                    .accept()
                    .await
                    .expect("accept pane list connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("accept pane list websocket");
                assert_eq!(next_frontend_kind(&mut socket).await, "list_windows");
                // The stalled event loop answers late, not never.
                tokio::time::sleep(Duration::from_millis(2_500)).await;
                let state = workspace_state_for_test(
                    project_root,
                    vec![window(
                        "tab-project::agent-project",
                        WindowPreset::Agent,
                        Some("codex"),
                    )],
                );
                socket
                    .send(Message::Text(state.to_string().into()))
                    .await
                    .expect("send pane list workspace state");
                // The caller drops the socket once it has the list, so a Close
                // frame — or nothing at all — means it never fell back.
                match tokio::time::timeout(Duration::from_millis(200), socket.next()).await {
                    Err(_) | Ok(None) => true,
                    Ok(Some(frame)) => !matches!(frame, Ok(Message::Text(_) | Message::Binary(_))),
                }
            });

            let windows =
                request_window_list(&format!("ws://{address}/internal/pane-ws"), project_root)
                    .await
                    .expect("pane list must outlast a multi-second backend stall");
            let stayed_on_the_light_route = server.await.expect("pane list mock task");

            assert_eq!(windows.len(), 1);
            assert!(
                stayed_on_the_light_route,
                "a slow backend must not push pane list onto the heavy sync request"
            );
        });
    }

    /// Issue #3510: `gwtd` and the running GUI can disagree about the pane
    /// protocol whenever the app is upgraded in place but not restarted. A
    /// backend from before the lightweight route silently drops
    /// `list_windows`, so `pane.list` must still answer instead of failing
    /// for the whole upgrade window.
    #[test]
    fn request_window_list_falls_back_to_frontend_ready_on_older_backends() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "pane-list-capability",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane list test runtime");

        runtime.block_on(async {
            let project_root = "/repo/project";
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind pane list mock");
            let address = listener.local_addr().expect("pane list mock address");
            let server = tokio::spawn(async move {
                let (stream, _) = listener
                    .accept()
                    .await
                    .expect("accept pane list connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("accept pane list websocket");
                // An older backend rejects the unknown request without any
                // reply, exactly as `AgentPaneSessionScope::filter_inbound`
                // used to.
                let mut kinds = vec![next_frontend_kind(&mut socket).await];
                kinds.push(next_frontend_kind(&mut socket).await);
                let state = workspace_state_for_test(
                    project_root,
                    vec![window(
                        "tab-project::agent-project",
                        WindowPreset::Agent,
                        Some("codex"),
                    )],
                );
                socket
                    .send(Message::Text(state.to_string().into()))
                    .await
                    .expect("send pane list workspace state");
                kinds
            });

            let windows = request_window_list_with_timeout(
                &format!("ws://{address}/internal/pane-ws"),
                project_root,
                Duration::from_millis(50),
            )
            .await
            .expect("pane list must answer through the compatibility fallback");
            let kinds = server.await.expect("pane list mock task");

            assert_eq!(kinds, vec!["list_windows", "frontend_ready"]);
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0].id, "tab-project::agent-project");
        });
    }

    #[test]
    fn issue_monitor_scan_client_sends_pathless_request_and_honors_ack() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "agent-capability-secret-sentinel",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build scan client test runtime");

        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind scan client mock");
            let address = listener.local_addr().expect("scan client mock address");
            let server = tokio::spawn(async move {
                for (accepted, reason) in [(true, None), (false, Some("scan_already_in_flight"))] {
                    let (stream, _) = listener.accept().await.expect("accept scan client");
                    let mut socket = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("accept scan client websocket");
                    let request = next_frontend_json(&mut socket).await;
                    assert_eq!(
                        request,
                        json!({
                            "kind": "agent_issue_monitor_scan_now",
                            "expected_project_scope": "scope-123",
                        })
                    );
                    assert!(
                        request.get("project_root").is_none(),
                        "the client must not claim project scope"
                    );
                    socket
                        .send(Message::Text(
                            json!({
                                "kind": "issue_monitor_scan_request_result",
                                "accepted": accepted,
                                "reason": reason,
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .expect("send scan request result");
                }
            });
            let ws_url = format!("ws://{address}/internal/pane-ws");

            assert_eq!(
                request_issue_monitor_scan_now_async(&ws_url, "scope-123").await,
                Ok(())
            );
            assert_eq!(
                request_issue_monitor_scan_now_async(&ws_url, "scope-123").await,
                Err("scan_already_in_flight".to_string())
            );
            server.await.expect("scan client mock task");
        });
    }

    #[test]
    fn issue_monitor_scan_client_marks_post_send_response_loss_as_unknown() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "agent-capability-secret-sentinel",
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build scan client test runtime");

        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind scan client mock");
            let address = listener.local_addr().expect("scan client mock address");
            let server = tokio::spawn(async move {
                for close in [
                    None,
                    Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: CloseCode::Policy,
                        reason: "stale capability".into(),
                    }),
                ] {
                    let (stream, _) = listener.accept().await.expect("accept scan client");
                    let mut socket = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("accept scan client websocket");
                    let request = next_frontend_json(&mut socket).await;
                    assert_eq!(request["expected_project_scope"], "scope-123");
                    socket
                        .close(close)
                        .await
                        .expect("close after accepting request without a result");
                }
            });
            let ws_url = format!("ws://{address}/internal/pane-ws");

            assert_eq!(
                request_issue_monitor_scan_now_async(&ws_url, "scope-123").await,
                Err("scan_delivery_unknown".to_string())
            );
            assert_eq!(
                request_issue_monitor_scan_now_async(&ws_url, "scope-123").await,
                Err("gui_capability_rejected".to_string())
            );
            server.await.expect("scan client mock task");
        });
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
                vec!["list_windows", "frontend_ready", "close_window"],
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
    fn pm_message_result_wait_covers_the_server_acceptance_window() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind delayed PM result mock");
            let address = listener.local_addr().expect("PM result mock address");
            let server = tokio::spawn(async move {
                let (stream, _) = listener
                    .accept()
                    .await
                    .expect("accept PM result connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("accept PM result websocket");
                tokio::time::sleep(Duration::from_millis(2_100)).await;
                socket
                    .send(Message::Text(
                        json!({
                            "kind": "pm_message_send_result",
                            "operation_id": "00000000-0000-4000-8000-000000000001",
                            "status": "delivered",
                            "window_id": "tab::agent",
                            "reason": null,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .expect("send delayed PM result");
            });
            let (mut socket, _) = connect_async(format!("ws://{address}/internal/pane-ws"))
                .await
                .expect("connect delayed PM result websocket");

            let reply = wait_for_pm_message_send_result(
                &mut socket,
                "00000000-0000-4000-8000-000000000001",
                "tab::agent",
                Duration::from_millis(2_500),
            )
            .await
            .expect("server result inside its acceptance window");

            assert_eq!(reply.status, "delivered");
            server.await.expect("PM result mock task");
        });
    }

    #[test]
    fn pm_message_transport_replay_reuses_exact_operation_and_target() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, "pm-capability");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build pane test runtime");

        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind PM replay mock");
            let address = listener.local_addr().expect("PM replay mock address");
            let mut target = window("tab::codex-1", WindowPreset::Codex, Some("codex"));
            target.session_id = Some("codex-session".to_string());
            let workspace = workspace_state_for_test("/repo/pm", vec![target]);
            let server = tokio::spawn(async move {
                let mut mutations = Vec::new();
                for attempt in 0..2 {
                    let (stream, _) = listener.accept().await.expect("accept PM connection");
                    let mut socket = tokio_tungstenite::accept_async(stream)
                        .await
                        .expect("accept PM websocket");
                    assert_eq!(next_frontend_kind(&mut socket).await, "frontend_ready");
                    socket
                        .send(Message::Text(workspace.to_string().into()))
                        .await
                        .expect("send workspace state");
                    let mutation = next_frontend_json(&mut socket).await;
                    mutations.push(mutation.clone());
                    if attempt == 0 {
                        socket.close(None).await.expect("drop first response");
                    } else {
                        socket
                            .send(Message::Text(
                                json!({
                                    "kind": "pm_message_send_result",
                                    "operation_id": mutation["operation_id"],
                                    "status": "delivered",
                                    "window_id": mutation["window_id"],
                                    "reason": null,
                                })
                                .to_string()
                                .into(),
                            ))
                            .await
                            .expect("send replay result");
                    }
                }
                mutations
            });
            let result = send_pm_pane_input(
                &format!("ws://{address}/internal/pane-ws"),
                None,
                "codex-1",
                "same operation body",
            )
            .await;
            let mutations = server.await.expect("PM replay mock task");

            assert_eq!(
                result,
                Ok("pm message delivered to tab::codex-1\n".to_string())
            );
            assert_eq!(mutations.len(), 2);
            assert_eq!(mutations[0]["kind"], "pm_pane_send_input");
            assert_eq!(mutations[0]["operation_id"], mutations[1]["operation_id"]);
            assert_eq!(mutations[0]["window_id"], mutations[1]["window_id"]);
            assert_eq!(mutations[0]["text"], mutations[1]["text"]);
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
                        "list_windows",
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
                vec!["list_windows", "frontend_ready", "close_window"]
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
                    "list_windows",
                    "frontend_ready",
                    "close_window",
                    "frontend_ready"
                ],
                "non-self close must retain authoritative post-close readback"
            );
        });
    }
}
