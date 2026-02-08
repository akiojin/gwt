//! Terminal/PTY management commands for xterm.js integration

use crate::state::AppState;
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::terminal::{AgentColor, BuiltinLaunchConfig};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};

/// Terminal output event payload sent to the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalOutputPayload {
    pub pane_id: String,
    pub data: Vec<u8>,
}

/// Serializable terminal info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct TerminalInfo {
    pub pane_id: String,
    pub agent_name: String,
    pub branch_name: String,
    pub status: String,
}

/// Launch a new terminal pane with an agent
#[tauri::command]
pub fn launch_terminal(
    agent_name: String,
    branch: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let working_dir = {
        let project_path = state
            .project_path
            .lock()
            .map_err(|e| format!("Failed to lock state: {}", e))?;
        match project_path.as_ref() {
            Some(p) => PathBuf::from(p),
            None => return Err("No project opened".to_string()),
        }
    };

    let config = BuiltinLaunchConfig {
        command: agent_name.clone(),
        args: vec![],
        working_dir,
        branch_name: branch,
        agent_name,
        agent_color: AgentColor::Green,
        env_vars: HashMap::new(),
    };

    let pane_id = {
        let mut manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
        manager
            .launch_agent(config, 24, 80)
            .map_err(|e| format!("Failed to launch terminal: {}", e))?
    };

    // Take the PTY reader and spawn a thread to stream output to the frontend
    let reader = {
        let manager = state
            .pane_manager
            .lock()
            .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
        let pane = manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .ok_or_else(|| "Pane not found after creation".to_string())?;
        pane.take_reader()
            .map_err(|e| format!("Failed to take reader: {}", e))?
    };

    let pane_id_clone = pane_id.clone();
    std::thread::spawn(move || {
        stream_pty_output(reader, pane_id_clone, app_handle);
    });

    Ok(pane_id)
}

/// Stream PTY output to the frontend via Tauri events
fn stream_pty_output(mut reader: Box<dyn Read + Send>, pane_id: String, app_handle: AppHandle) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let payload = TerminalOutputPayload {
                    pane_id: pane_id.clone(),
                    data: buf[..n].to_vec(),
                };
                if app_handle.emit("terminal-output", &payload).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Write data to a terminal pane
#[tauri::command]
pub fn write_terminal(
    pane_id: String,
    data: Vec<u8>,
    state: State<AppState>,
) -> Result<(), String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
    let pane = manager
        .pane_mut_by_id(&pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;
    pane.write_input(&data)
        .map_err(|e| format!("Failed to write to terminal: {}", e))
}

/// Resize a terminal pane
#[tauri::command]
pub fn resize_terminal(
    pane_id: String,
    rows: u16,
    cols: u16,
    state: State<AppState>,
) -> Result<(), String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;
    let pane = manager
        .pane_mut_by_id(&pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;
    pane.resize(rows, cols)
        .map_err(|e| format!("Failed to resize terminal: {}", e))
}

/// Close a terminal pane
#[tauri::command]
pub fn close_terminal(pane_id: String, state: State<AppState>) -> Result<(), String> {
    let mut manager = state
        .pane_manager
        .lock()
        .map_err(|e| format!("Failed to lock pane manager: {}", e))?;

    let index = manager
        .panes()
        .iter()
        .position(|p| p.pane_id() == pane_id)
        .ok_or_else(|| format!("Pane not found: {}", pane_id))?;

    manager.close_pane(index);
    Ok(())
}

/// List all active terminal panes
#[tauri::command]
pub fn list_terminals(state: State<AppState>) -> Vec<TerminalInfo> {
    let manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    manager
        .panes()
        .iter()
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
        .collect()
}
