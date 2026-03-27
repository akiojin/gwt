use axum::{extract::State, response::IntoResponse, Json};
use gwt_core::terminal::pane::PaneStatus;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::state::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTerminalsRequest {
    pub project_root: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub pane_id: String,
    pub agent_name: String,
    pub branch_name: String,
    pub status: String,
}

pub async fn list_terminals(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListTerminalsRequest>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        let manager = match state.pane_manager.lock() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };

        let project_filter = req.project_root.map(PathBuf::from);
        manager
            .panes()
            .iter()
            .filter(|pane| match &project_filter {
                Some(root) => pane.project_root() == root.as_path(),
                None => true,
            })
            .map(|pane| {
                let status = match pane.status() {
                    PaneStatus::Running => "running".to_string(),
                    PaneStatus::Completed(code) => format!("completed({})", code),
                    PaneStatus::Error(msg) => format!("error: {}", msg),
                };
                TerminalInfo {
                    pane_id: pane.pane_id().to_string(),
                    agent_name: pane.agent_name().to_string(),
                    branch_name: pane.branch_name().to_string(),
                    status,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();

    Json(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTerminalRequest {
    pub pane_id: String,
    pub data: String,
}

pub async fn write_terminal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WriteTerminalRequest>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|_| "Failed to acquire pane manager lock".to_string())?;
        let pane = manager
            .pane_mut_by_id(&req.pane_id)
            .ok_or_else(|| format!("Pane not found: {}", req.pane_id))?;
        pane.write_input(req.data.as_bytes())
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or(Err("Task join error".to_string()));

    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeTerminalRequest {
    pub pane_id: String,
    pub cols: u16,
    pub rows: u16,
}

pub async fn resize_terminal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ResizeTerminalRequest>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|_| "Failed to acquire pane manager lock".to_string())?;
        let pane = manager
            .pane_mut_by_id(&req.pane_id)
            .ok_or_else(|| format!("Pane not found: {}", req.pane_id))?;
        pane.resize(req.rows, req.cols).map_err(|e| e.to_string())
    })
    .await
    .unwrap_or(Err("Task join error".to_string()));

    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTerminalRequest {
    pub pane_id: String,
}

pub async fn close_terminal(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CloseTerminalRequest>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|_| "Failed to acquire pane manager lock".to_string())?;
        manager.remove_pane(&req.pane_id);
        Ok(())
    })
    .await
    .unwrap_or(Err("Task join error".to_string()));

    match result {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": e })),
        )
            .into_response(),
    }
}
