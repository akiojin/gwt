//! MCP tool handler implementations for the WebSocket server.
//!
//! Each handler receives the WsContext and JSON-RPC params, and returns
//! a JSON-RPC response value or error.

use crate::mcp_ws_server::{JsonRpcResponse, WsContext};
use crate::state::AppState;
use gwt_core::terminal::pane::PaneStatus;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Rate limiting for gwt_launch_agent
// ---------------------------------------------------------------------------

const MAX_TABS: usize = 8;
const RATE_WINDOW_SECS: u64 = 60;
const RATE_LIMIT: u64 = 5;

static LAUNCH_COUNTER: AtomicU64 = AtomicU64::new(0);
static LAUNCH_WINDOW_START: AtomicU64 = AtomicU64::new(0);

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

    let tabs: Vec<Value> = manager
        .panes_mut()
        .iter_mut()
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

    let formatted = format!("[gwt msg from {sender}]: {sanitized}\n");

    let state = ctx.app_handle.state::<AppState>();
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
        "MCP launch agent request"
    );

    // Return a placeholder response. The actual launch requires the frontend
    // to orchestrate worktree resolution, Docker checks, etc. via the existing
    // launch_agent / start_launch_job flow. MCP can trigger this by emitting
    // an event that the frontend handles.
    //
    // For now, emit the event and return the request acknowledgement.
    let _ = ctx.app_handle.emit(
        "mcp-launch-agent",
        serde_json::json!({
            "agent_id": agent_id,
            "branch": branch,
        }),
    );

    JsonRpcResponse::success(
        id,
        serde_json::json!({
            "status": "requested",
            "agent_id": agent_id,
            "branch": branch,
        }),
    )
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

pub fn handle_get_worktree_diff(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
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

    // Run `git diff` in the worktree directory
    let output = match std::process::Command::new("git")
        .args(["diff"])
        .current_dir(&worktree_path)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            return JsonRpcResponse::error(id, -32603, format!("Failed to run git diff: {e}"));
        }
    };

    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    JsonRpcResponse::success(id, serde_json::json!({ "diff": diff }))
}

// ---------------------------------------------------------------------------
// gwt_get_changed_files (FR-014)
// ---------------------------------------------------------------------------

pub fn handle_get_changed_files(id: Value, params: &Value, ctx: &WsContext) -> JsonRpcResponse {
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

    match gwt_core::git::get_working_tree_status(&worktree_path) {
        Ok(entries) => {
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
        Err(e) => JsonRpcResponse::error(id, -32603, format!("Failed to get changed files: {e}")),
    }
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
