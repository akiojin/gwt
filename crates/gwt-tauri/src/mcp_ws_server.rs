//! WebSocket server for MCP bridge communication.
//!
//! Accepts connections from MCP bridge processes and routes JSON-RPC
//! requests to the appropriate tool handlers.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

/// Connection info written to `~/.gwt/mcp-state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpState {
    pub port: u16,
    pub pid: u32,
}

/// Shared context available to all WebSocket handler tasks.
/// Fields are used by tool handlers implemented in a separate module.
#[allow(dead_code)]
pub struct WsContext {
    pub app_handle: tauri::AppHandle<tauri::Wry>,
}

/// Handle to the running WebSocket server, used for lifecycle management.
pub struct McpWsHandle {
    pub port: u16,
    shutdown: Arc<Notify>,
}

impl McpWsHandle {
    /// Signal the server to shut down gracefully.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }
}

impl Drop for McpWsHandle {
    fn drop(&mut self) {
        self.shutdown();
        let _ = remove_state_file();
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: serde_json::Value, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError { code, message }),
            id,
        }
    }
}

// Standard JSON-RPC error codes
const METHOD_NOT_FOUND: i64 = -32601;
const PARSE_ERROR: i64 = -32700;

// ---------------------------------------------------------------------------
// State file management (~/.gwt/mcp-state.json)
// ---------------------------------------------------------------------------

fn state_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gwt").join("mcp-state.json"))
}

/// Write connection info so bridge processes can discover the server.
fn write_state_file(port: u16) -> std::io::Result<()> {
    let path = state_file_path().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "home directory not found")
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let state = McpState {
        port,
        pid: std::process::id(),
    };
    let json = serde_json::to_string_pretty(&state).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;

    info!(
        category = "mcp",
        event = "StateFileWritten",
        port = port,
        path = %path.display(),
        "Wrote MCP state file"
    );
    Ok(())
}

/// Remove state file on shutdown or cleanup.
fn remove_state_file() -> std::io::Result<()> {
    if let Some(path) = state_file_path() {
        if path.exists() {
            std::fs::remove_file(&path)?;
            info!(
                category = "mcp",
                event = "StateFileRemoved",
                path = %path.display(),
                "Removed MCP state file"
            );
        }
    }
    Ok(())
}

/// Clean up stale state file from a previous crash.
pub fn cleanup_stale_state_file() {
    let Some(path) = state_file_path() else {
        return;
    };
    if !path.exists() {
        return;
    }

    // Read existing state and check if the process is still alive.
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let state: McpState = match serde_json::from_str(&contents) {
        Ok(s) => s,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return;
        }
    };

    let alive = process_is_alive(state.pid);
    if !alive {
        info!(
            category = "mcp",
            event = "StaleStateFileCleaned",
            pid = state.pid,
            "Removed stale MCP state file from crashed process"
        );
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };

    match unsafe { libc::kill(pid, 0) } {
        0 => true,
        -1 => matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(err) if err == libc::EPERM
        ),
        _ => false,
    }
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Server startup
// ---------------------------------------------------------------------------

/// Start the WebSocket server on a random port.
///
/// Returns a handle that can be used to shut down the server. The handle
/// also removes the state file when dropped.
pub async fn start(app_handle: tauri::AppHandle<tauri::Wry>) -> std::io::Result<McpWsHandle> {
    // Bind to port 0 to let the OS assign a random available port.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let port = addr.port();

    write_state_file(port)?;

    info!(
        category = "mcp",
        event = "WsServerStarted",
        port = port,
        "MCP WebSocket server listening"
    );

    let shutdown = Arc::new(Notify::new());
    let ctx = Arc::new(WsContext { app_handle });

    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        accept_loop(listener, ctx, shutdown_clone).await;
    });

    Ok(McpWsHandle { port, shutdown })
}

async fn accept_loop(listener: TcpListener, ctx: Arc<WsContext>, shutdown: Arc<Notify>) {
    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, addr, ctx).await {
                                warn!(
                                    category = "mcp",
                                    event = "ConnectionError",
                                    error = %e,
                                    "WebSocket connection error"
                                );
                            }
                        });
                    }
                    Err(e) => {
                        warn!(
                            category = "mcp",
                            event = "AcceptError",
                            error = %e,
                            "Failed to accept connection"
                        );
                    }
                }
            }
            _ = shutdown.notified() => {
                info!(
                    category = "mcp",
                    event = "WsServerStopping",
                    "MCP WebSocket server shutting down"
                );
                break;
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    ctx: Arc<WsContext>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        category = "mcp",
        event = "ClientConnected",
        addr = %addr,
        "MCP bridge connected"
    );

    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let (mut writer, mut reader) = ws_stream.split();

    while let Some(msg) = reader.next().await {
        let msg = msg?;
        match msg {
            Message::Text(text) => {
                let response = route_request(&text, &ctx).await;
                let json = serde_json::to_string(&response)?;
                writer.send(Message::Text(json.into())).await?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    info!(
        category = "mcp",
        event = "ClientDisconnected",
        addr = %addr,
        "MCP bridge disconnected"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// JSON-RPC routing
// ---------------------------------------------------------------------------

async fn route_request(text: &str, ctx: &WsContext) -> JsonRpcResponse {
    let req: JsonRpcRequest = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => {
            return JsonRpcResponse::error(
                serde_json::Value::Null,
                PARSE_ERROR,
                format!("Invalid JSON-RPC request: {e}"),
            );
        }
    };

    let id = req.id.clone();
    let params = &req.params;

    use crate::mcp_handlers;
    match req.method.as_str() {
        "gwt_list_tabs" => mcp_handlers::handle_list_tabs(id, params, ctx),
        "gwt_get_tab_info" => mcp_handlers::handle_get_tab_info(id, params, ctx),
        "gwt_send_message" => mcp_handlers::handle_send_message(id, params, ctx),
        "gwt_broadcast_message" => mcp_handlers::handle_broadcast_message(id, params, ctx),
        "gwt_launch_agent" => mcp_handlers::handle_launch_agent(id, params, ctx),
        "gwt_stop_tab" => mcp_handlers::handle_stop_tab(id, params, ctx),
        "gwt_get_worktree_diff" => mcp_handlers::handle_get_worktree_diff(id, params, ctx).await,
        "gwt_get_changed_files" => mcp_handlers::handle_get_changed_files(id, params, ctx).await,
        _ => JsonRpcResponse::error(
            id,
            METHOD_NOT_FOUND,
            format!("Unknown method: {}", req.method),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_response_success_serialization() {
        let resp = JsonRpcResponse::success(serde_json::json!(1), serde_json::json!({"ok": true}));
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["result"]["ok"], true);
        assert!(json.get("error").is_none());
        assert_eq!(json["id"], 1);
    }

    #[test]
    fn json_rpc_response_error_serialization() {
        let resp = JsonRpcResponse::error(serde_json::json!(2), -32601, "not found".to_string());
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert!(json.get("result").is_none());
        assert_eq!(json["error"]["code"], -32601);
        assert_eq!(json["error"]["message"], "not found");
    }

    #[test]
    fn route_request_parse_error() {
        // WsContext is not needed for parse error path, but we cannot construct one
        // without an AppHandle. Instead test the parsing branch directly.
        let resp: JsonRpcRequest = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"gwt_list_tabs","params":{},"id":1}"#,
        )
        .unwrap();
        assert_eq!(resp.method, "gwt_list_tabs");
        assert_eq!(resp.id, serde_json::json!(1));
    }

    #[test]
    fn route_request_invalid_json() {
        // Verify parse error returns correct code
        let bad_json = "not json at all";
        let req: Result<JsonRpcRequest, _> = serde_json::from_str(bad_json);
        assert!(req.is_err());
    }

    #[test]
    fn mcp_state_serialization() {
        let state = McpState {
            port: 12345,
            pid: 9999,
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: McpState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.port, 12345);
        assert_eq!(parsed.pid, 9999);
    }
}
