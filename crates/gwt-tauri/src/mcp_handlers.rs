//! MCP tool handler implementations for the WebSocket server.
//!
//! Each handler receives the WsContext and JSON-RPC params, and returns
//! a JSON-RPC response value or error.

use crate::agent_master::{get_agent_mode_state, send_agent_message, AgentModeState};
use crate::commands::terminal::{launch_agent_for_project_root, LaunchAgentRequest};
use crate::mcp_ws_server::{JsonRpcResponse, WsContext};
use crate::state::AppState;
use gwt_core::config::{get_mcp_registration_status, repair_mcp_registration};
use gwt_core::terminal::pane::PaneStatus;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Rate limiting for gwt_launch_agent
// ---------------------------------------------------------------------------

const MAX_TABS: usize = 8;
const RATE_WINDOW_SECS: u64 = 60;
const RATE_LIMIT: u64 = 5;

static LAUNCH_COUNTER: AtomicU64 = AtomicU64::new(0);
static LAUNCH_WINDOW_START: AtomicU64 = AtomicU64::new(0);
static MASTER_COMMAND_COUNTER: AtomicU64 = AtomicU64::new(0);

fn check_rate_limit() -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let window_start = LAUNCH_WINDOW_START.load(Ordering::SeqCst);
    if now - window_start >= RATE_WINDOW_SECS {
        LAUNCH_WINDOW_START.store(now, Ordering::SeqCst);
        LAUNCH_COUNTER.store(1, Ordering::SeqCst);
        return Ok(());
    }

    let count = LAUNCH_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    if count > RATE_LIMIT {
        return Err(format!(
            "Rate limit exceeded: max {} launches per {} seconds",
            RATE_LIMIT, RATE_WINDOW_SECS
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: sanitize message text (strip control characters)
// ---------------------------------------------------------------------------

fn sanitize_message(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

fn get_string_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, JsonRpcResponse> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            JsonRpcResponse::error(
                Value::Null,
                -32602,
                format!("Missing or empty required parameter: {key}"),
            )
        })
}

fn get_master_window_label(ctx: &WsContext, state: &AppState) -> String {
    let focused = ctx
        .app_handle
        .webview_windows()
        .into_iter()
        .find_map(|(label, window)| {
            window
                .is_focused()
                .ok()
                .and_then(|focused| focused.then_some(label))
        });

    focused
        .or_else(|| state.project_for_window("main").map(|_| "main".to_string()))
        .or_else(|| {
            ctx.app_handle
                .webview_windows()
                .into_iter()
                .next()
                .map(|(label, _)| label)
        })
        .unwrap_or_else(|| "main".to_string())
}

fn next_master_command_id() -> String {
    format!(
        "mcmd-{}",
        MASTER_COMMAND_COUNTER.fetch_add(1, Ordering::SeqCst) + 1
    )
}

fn refresh_mcp_registration_health(
    ctx: &WsContext,
    state: &AppState,
) -> gwt_core::config::McpRegistrationStatus {
    let resource_dir = ctx.app_handle.path().resource_dir().ok();
    let status = get_mcp_registration_status(resource_dir.as_deref());
    state.set_mcp_registration_status(status.clone());
    status
}

fn maybe_master_mcp_guard(id: Value, ctx: &WsContext, state: &AppState) -> Option<JsonRpcResponse> {
    let status = refresh_mcp_registration_health(ctx, state);
    if status.overall == "ok" {
        return None;
    }
    let reason = if status.bridge_runtime != "ok" {
        "MCP_RUNTIME_MISSING"
    } else if status.bridge_script != "ok" {
        "MCP_BRIDGE_SCRIPT_MISSING"
    } else {
        "MCP_REGISTER_MISSING"
    };

    Some(JsonRpcResponse::success(
        id,
        serde_json::json!({
            "accepted": false,
            "status": "rejected",
            "reason": format!("mcp_registration_failed:{reason}"),
            "suggested_action": "repair_mcp_registration"
        }),
    ))
}

// ---------------------------------------------------------------------------
// gwt_list_tabs
// ---------------------------------------------------------------------------

pub fn handle_list_tabs(id: Value, _params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let state = ctx.app_handle.state::<AppState>();
    let mut manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Internal error: {e}"));
        }
    };

    // Project isolation: determine the caller's project root from MCP context.
    let window_label = get_master_window_label(ctx, &state);
    let project_filter = state
        .project_for_window(&window_label)
        .map(PathBuf::from);

    let tabs: Vec<Value> = manager
        .panes_mut()
        .iter_mut()
        .filter(|pane| match &project_filter {
            Some(root) => pane.project_root() == root.as_path(),
            None => true,
        })
        .map(|pane| {
            let _ = pane.check_status();
            let status = match pane.status() {
                PaneStatus::Running => "running",
                PaneStatus::Completed(_) => "completed",
                PaneStatus::Error(_) => "error",
            };
            serde_json::json!({
                "tab_id": pane.pane_id(),
                "agent_type": pane.agent_name(),
                "branch": pane.branch_name(),
                "status": status,
            })
        })
        .collect();

    JsonRpcResponse::success(id, Value::Array(tabs))
}

// ---------------------------------------------------------------------------
// gwt_get_tab_info
// ---------------------------------------------------------------------------

pub fn handle_get_tab_info(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let tab_id = match get_string_param(params, "tab_id") {
        Ok(v) => v,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let state = ctx.app_handle.state::<AppState>();
    let mut manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Internal error: {e}"));
        }
    };

    let pane = match manager.pane_mut_by_id(tab_id) {
        Some(p) => p,
        None => {
            return JsonRpcResponse::error(id, -32604, format!("Tab not found: {tab_id}"));
        }
    };

    let _ = pane.check_status();
    let status = match pane.status() {
        PaneStatus::Running => "running",
        PaneStatus::Completed(_) => "completed",
        PaneStatus::Error(_) => "error",
    };

    // Gather extra info from launch metadata if available.
    let meta = state
        .pane_launch_meta
        .lock()
        .ok()
        .and_then(|m| m.get(tab_id).cloned());

    let mut info = serde_json::json!({
        "tab_id": pane.pane_id(),
        "agent_type": pane.agent_name(),
        "branch": pane.branch_name(),
        "status": status,
    });

    if let Some(meta) = meta {
        if let Some(obj) = info.as_object_mut() {
            obj.insert(
                "worktree_path".to_string(),
                Value::String(meta.worktree_path.to_string_lossy().to_string()),
            );
        }
    }

    JsonRpcResponse::success(id, info)
}

// ---------------------------------------------------------------------------
// gwt_send_message (FR-009, FR-015)
// ---------------------------------------------------------------------------

pub fn handle_send_message(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let target_tab_id = match get_string_param(params, "target_tab_id") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };
    let message = match get_string_param(params, "message") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let sanitized = sanitize_message(&message);
    let sender = params
        .get("sender")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let sender = sanitize_message(sender);

    let formatted = format!("[gwt msg from {sender}]: {sanitized}\n");

    let state = ctx.app_handle.state::<AppState>();

    // Project isolation: determine the caller's project root from MCP context.
    let window_label = get_master_window_label(ctx, &state);
    let project_filter = state
        .project_for_window(&window_label)
        .map(PathBuf::from);

    let mut manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Internal error: {e}"));
        }
    };

    let pane = match manager.pane_mut_by_id(&target_tab_id) {
        Some(p) => p,
        None => {
            return JsonRpcResponse::error(id, -32604, format!("Tab not found: {target_tab_id}"));
        }
    };

    // Reject access to panes belonging to a different project.
    if let Some(ref root) = project_filter {
        if pane.project_root() != root.as_path() {
            return JsonRpcResponse::error(
                id,
                -32604,
                format!("Access denied: tab {} belongs to a different project", target_tab_id),
            );
        }
    }

    let _ = pane.check_status();
    if !matches!(pane.status(), PaneStatus::Running) {
        return JsonRpcResponse::error(id, -32605, format!("Tab not running: {target_tab_id}"));
    }

    if let Err(e) = pane.write_input(formatted.as_bytes()) {
        return JsonRpcResponse::error(id, -32603, format!("Failed to send message: {e}"));
    }

    info!(
        category = "mcp",
        event = "MessageSent",
        target = %target_tab_id,
        "MCP message sent to tab"
    );

    JsonRpcResponse::success(id, serde_json::json!({ "success": true }))
}

// ---------------------------------------------------------------------------
// gwt_broadcast_message (FR-010)
// ---------------------------------------------------------------------------

pub fn handle_broadcast_message(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let message = match get_string_param(params, "message") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let sanitized = sanitize_message(&message);
    let sender = params
        .get("sender")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let sender = sanitize_message(sender);

    let formatted = format!("[gwt msg from {sender}]: {sanitized}\n");

    let state = ctx.app_handle.state::<AppState>();
    let mut manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Internal error: {e}"));
        }
    };

    let sender_tab_id = params.get("sender_tab_id").and_then(|v| v.as_str());

    let mut sent_count = 0u64;
    for pane in manager.panes_mut() {
        // Skip the sender
        if let Some(sender_id) = sender_tab_id {
            if pane.pane_id() == sender_id {
                continue;
            }
        }

        let _ = pane.check_status();
        if !matches!(pane.status(), PaneStatus::Running) {
            continue;
        }

        if pane.write_input(formatted.as_bytes()).is_ok() {
            sent_count += 1;
        }
    }

    info!(
        category = "mcp",
        event = "BroadcastSent",
        count = sent_count,
        "MCP broadcast message sent"
    );

    JsonRpcResponse::success(id, serde_json::json!({ "sent_count": sent_count }))
}

// ---------------------------------------------------------------------------
// gwt_launch_agent (FR-011, FR-016)
// ---------------------------------------------------------------------------

pub fn handle_launch_agent(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let agent_id = match get_string_param(params, "agent_id") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };
    let branch = match get_string_param(params, "branch") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    // Check tab count limit (FR-016)
    let state = ctx.app_handle.state::<AppState>();
    {
        let mut manager = match state.pane_manager.lock() {
            Ok(m) => m,
            Err(e) => {
                return JsonRpcResponse::error(id, -32603, format!("Internal error: {e}"));
            }
        };

        let running_count = manager.panes_mut().iter_mut().fold(0usize, |acc, p| {
            let _ = p.check_status();
            if matches!(p.status(), PaneStatus::Running) {
                acc + 1
            } else {
                acc
            }
        });

        if running_count >= MAX_TABS {
            return JsonRpcResponse::error(
                id,
                -32606,
                format!("Tab limit reached: max {MAX_TABS} running tabs"),
            );
        }
    }

    // Check rate limit (FR-016)
    if let Err(e) = check_rate_limit() {
        return JsonRpcResponse::error(id, -32607, e);
    }

    info!(
        category = "mcp",
        event = "LaunchAgentRequested",
        agent_id = %agent_id,
        branch = %branch,
        "MCP launch agent requested"
    );

    let state = ctx.app_handle.state::<AppState>();

    // Resolve a project context from the active/known windows.
    let maybe_project_root = {
        let focused_label =
            ctx.app_handle
                .webview_windows()
                .into_iter()
                .find_map(|(label, window)| {
                    window
                        .is_focused()
                        .ok()
                        .and_then(|focused| focused.then_some(label))
                });

        focused_label
            .as_ref()
            .and_then(|label| state.project_for_window(label))
            .or_else(|| state.project_for_window("main"))
            .or_else(|| {
                state
                    .window_projects
                    .lock()
                    .ok()
                    .and_then(|projects| projects.values().next().cloned())
            })
    };

    let project_root = match maybe_project_root {
        Some(path) => PathBuf::from(path),
        None => {
            return JsonRpcResponse::error(
                id,
                -32608,
                "No project opened for MCP launch".to_string(),
            );
        }
    };

    let request = LaunchAgentRequest {
        agent_id: agent_id.clone(),
        branch: branch.clone(),
        profile: None,
        model: None,
        agent_version: None,
        mode: None,
        skip_permissions: None,
        reasoning_level: None,
        fast_mode: None,
        collaboration_modes: None,
        extra_args: None,
        env_overrides: None,
        docker_service: None,
        docker_force_host: None,
        docker_recreate: None,
        docker_build: None,
        docker_keep: None,
        resume_session_id: None,
        create_branch: None,
        issue_number: None,
        ai_branch_description: None,
        terminal_shell: None,
    };

    let tab_id = match launch_agent_for_project_root(
        project_root,
        request,
        &state,
        ctx.app_handle.clone(),
        None,
        None,
    ) {
        Ok(tab_id) => tab_id,
        Err(err) => {
            warn!(
                category = "mcp",
                event = "LaunchAgentFailed",
                agent_id = %agent_id,
                branch = %branch,
                error = %err,
                "Failed to launch via MCP"
            );
            return JsonRpcResponse::error(id, -32603, format!("Failed to launch agent: {err}"));
        }
    };

    info!(
        category = "mcp",
        event = "LaunchAgentStarted",
        agent_id = %agent_id,
        branch = %branch,
        tab_id = %tab_id,
        "MCP launch agent started"
    );

    // NOTE: keep "tab_id" for compatibility with MCP tool contract.
    JsonRpcResponse::success(id, serde_json::json!({ "tab_id": tab_id }))
}

// ---------------------------------------------------------------------------
// gwt_stop_tab (FR-012)
// ---------------------------------------------------------------------------

pub fn handle_stop_tab(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let tab_id = match get_string_param(params, "tab_id") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let state = ctx.app_handle.state::<AppState>();
    let mut manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Internal error: {e}"));
        }
    };

    let pane = match manager.pane_mut_by_id(&tab_id) {
        Some(p) => p,
        None => {
            return JsonRpcResponse::error(id, -32604, format!("Tab not found: {tab_id}"));
        }
    };

    let _ = pane.check_status();
    if !matches!(pane.status(), PaneStatus::Running) {
        return JsonRpcResponse::error(id, -32605, format!("Tab not running: {tab_id}"));
    }

    if let Err(e) = pane.kill() {
        warn!(
            category = "mcp",
            event = "StopTabFailed",
            tab_id = %tab_id,
            error = %e,
            "Failed to stop tab"
        );
        return JsonRpcResponse::error(id, -32603, format!("Failed to stop tab: {e}"));
    }

    info!(
        category = "mcp",
        event = "TabStopped",
        tab_id = %tab_id,
        "Tab stopped via MCP"
    );

    JsonRpcResponse::success(id, serde_json::json!({ "success": true }))
}

// ---------------------------------------------------------------------------
// gwt_get_worktree_diff (FR-013)
// ---------------------------------------------------------------------------

pub async fn handle_get_worktree_diff(
    id: Value,
    params: &Value,
    ctx: &WsContext,
) -> JsonRpcResponse {
    let tab_id = match get_string_param(params, "tab_id") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let worktree_path = match resolve_worktree_path_for_tab(&tab_id, ctx) {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::error(id, -32604, e),
    };

    let output = match tokio::task::spawn_blocking(move || {
        gwt_core::process::command("git")
            .args(["diff"])
            .current_dir(&worktree_path)
            .output()
    })
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return JsonRpcResponse::error(id, -32603, format!("Failed to run git diff: {e}"));
        }
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Git diff task failed: {e}"));
        }
    };

    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    JsonRpcResponse::success(id, serde_json::json!({ "diff": diff }))
}

// ---------------------------------------------------------------------------
// gwt_get_changed_files (FR-014)
// ---------------------------------------------------------------------------

pub async fn handle_get_changed_files(
    id: Value,
    params: &Value,
    ctx: &WsContext,
) -> JsonRpcResponse {
    let tab_id = match get_string_param(params, "tab_id") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let worktree_path = match resolve_worktree_path_for_tab(&tab_id, ctx) {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::error(id, -32604, e),
    };

    let entries = match tokio::task::spawn_blocking(move || {
        gwt_core::git::get_working_tree_status(&worktree_path)
    })
    .await
    {
        Ok(Ok(entries)) => entries,
        Ok(Err(e)) => {
            return JsonRpcResponse::error(id, -32603, format!("Failed to get changed files: {e}"));
        }
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Changed files task failed: {e}"));
        }
    };

    let files: Vec<Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "path": e.path,
                "status": format!("{:?}", e.status).to_lowercase(),
                "is_staged": e.is_staged,
            })
        })
        .collect();
    JsonRpcResponse::success(id, Value::Array(files))
}

// ---------------------------------------------------------------------------
// gwt_master_send
// ---------------------------------------------------------------------------

pub fn handle_master_send(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let state = ctx.app_handle.state::<AppState>();
    if let Some(resp) = maybe_master_mcp_guard(id.clone(), ctx, &state) {
        return resp;
    }

    let command_type = match get_string_param(params, "command_type") {
        Ok(v) => v.to_string(),
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };
    let command_id = next_master_command_id();
    let window_label = get_master_window_label(ctx, &state);

    match command_type.as_str() {
        "input_text" => {
            let payload_text = params
                .get("payload")
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let fallback_text = params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let text = if !payload_text.trim().is_empty() {
                payload_text
            } else {
                fallback_text
            };
            if text.trim().is_empty() {
                return JsonRpcResponse::success(
                    id,
                    serde_json::json!({
                        "accepted": false,
                        "command_id": command_id,
                        "status": "rejected",
                        "reason": "payload.text is required for input_text"
                    }),
                );
            }

            let app_handle = ctx.app_handle.clone();
            let input = sanitize_message(text);
            let window_label_clone = window_label.clone();
            std::thread::spawn(move || {
                let state = app_handle.state::<AppState>();
                let _ = send_agent_message(&state, &window_label_clone, &input);
            });

            JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "accepted": true,
                    "command_id": command_id,
                    "status": "queued"
                }),
            )
        }
        "clear_context" | "new_session" => {
            if let Ok(mut map) = state.window_agent_modes.lock() {
                map.insert(window_label, AgentModeState::new());
            }
            JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "accepted": true,
                    "command_id": command_id,
                    "status": "done"
                }),
            )
        }
        "cancel" => {
            if let Ok(mut map) = state.window_agent_modes.lock() {
                if let Some(mode) = map.get_mut(&window_label) {
                    mode.is_waiting = false;
                    mode.last_error = Some("Cancelled by MCP command.".to_string());
                }
            }
            JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "accepted": true,
                    "command_id": command_id,
                    "status": "done"
                }),
            )
        }
        "switch_session" => JsonRpcResponse::success(
            id,
            serde_json::json!({
                "accepted": false,
                "command_id": command_id,
                "status": "rejected",
                "reason": "switch_session is not supported yet"
            }),
        ),
        _ => JsonRpcResponse::success(
            id,
            serde_json::json!({
                "accepted": false,
                "command_id": command_id,
                "status": "rejected",
                "reason": format!("Unsupported command_type: {}", command_type)
            }),
        ),
    }
}

// ---------------------------------------------------------------------------
// gwt_master_get_state
// ---------------------------------------------------------------------------

pub fn handle_master_get_state(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
    let state = ctx.app_handle.state::<AppState>();
    let window_label = get_master_window_label(ctx, &state);
    let include_messages = params
        .get("include_messages")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(200) as usize;

    let mut mode = get_agent_mode_state(&state, &window_label);
    if include_messages {
        if mode.messages.len() > limit {
            let start = mode.messages.len().saturating_sub(limit);
            mode.messages = mode.messages.split_off(start);
        }
    } else {
        mode.messages.clear();
    }

    JsonRpcResponse::success(
        id,
        serde_json::to_value(mode).unwrap_or_else(|_| serde_json::json!({})),
    )
}

// ---------------------------------------------------------------------------
// gwt_master_get_mcp_registration_status
// ---------------------------------------------------------------------------

pub fn handle_master_get_mcp_registration_status(
    id: Value,
    _params: &Value,
    ctx: &WsContext,
) -> JsonRpcResponse {
    let state = ctx.app_handle.state::<AppState>();
    let status = refresh_mcp_registration_health(ctx, &state);
    JsonRpcResponse::success(
        id,
        serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})),
    )
}

// ---------------------------------------------------------------------------
// gwt_master_repair_mcp_registration
// ---------------------------------------------------------------------------

pub fn handle_master_repair_mcp_registration(
    id: Value,
    _params: &Value,
    ctx: &WsContext,
) -> JsonRpcResponse {
    crate::mcp_ws_server::cleanup_stale_state_file();
    let state = ctx.app_handle.state::<AppState>();
    let resource_dir = ctx.app_handle.path().resource_dir().ok();
    let status = repair_mcp_registration(resource_dir.as_deref());
    state.set_mcp_registration_status(status.clone());
    JsonRpcResponse::success(
        id,
        serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})),
    )
}

// ---------------------------------------------------------------------------
// Helper: resolve worktree path for a tab
// ---------------------------------------------------------------------------

fn resolve_worktree_path_for_tab(
    tab_id: &str,
    ctx: &WsContext,
) -> Result<std::path::PathBuf, String> {
    let state = ctx.app_handle.state::<AppState>();

    // First try launch metadata for the worktree path.
    if let Ok(meta_map) = state.pane_launch_meta.lock() {
        if let Some(meta) = meta_map.get(tab_id) {
            return Ok(meta.worktree_path.clone());
        }
    }

    // Fallback: the tab might not have metadata; return an error.
    Err(format!("No worktree path found for tab: {tab_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_message_strips_control_chars() {
        assert_eq!(sanitize_message("hello\x00world"), "helloworld");
        assert_eq!(sanitize_message("hello\nworld"), "hello\nworld");
        assert_eq!(sanitize_message("hello\tworld"), "hello\tworld");
        assert_eq!(sanitize_message("hello\x07\x1bworld"), "helloworld");
    }

    #[test]
    fn sanitize_message_preserves_normal_text() {
        let text = "fix completed on feature/auth (v2.0)";
        assert_eq!(sanitize_message(text), text);
    }

    #[test]
    fn rate_limit_resets_after_window() {
        // Reset state
        LAUNCH_WINDOW_START.store(0, Ordering::SeqCst);
        LAUNCH_COUNTER.store(0, Ordering::SeqCst);

        // First call should succeed and reset window
        assert!(check_rate_limit().is_ok());
    }
}
