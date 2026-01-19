//! tmux pane management
//!
//! Provides functions to create, list, and manage tmux panes for agents.

use std::process::Command;
use std::time::{Duration, SystemTime};

use super::error::{TmuxError, TmuxResult};

/// Information about a tmux pane
#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub pane_id: String,
    pub pane_pid: u32,
    pub current_command: String,
    /// Current working directory of the pane
    pub current_path: Option<String>,
}

/// Geometry information for a tmux pane
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneGeometry {
    pub pane_id: String,
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
}

/// Column grouping for panes aligned by left coordinate
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneColumn {
    pub left: u16,
    pub width: u16,
    pub pane_ids: Vec<String>,
    pub total_height: u16,
}

/// Split direction for tmux pane operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

impl SplitDirection {
    pub fn tmux_flag(self) -> &'static str {
        match self {
            SplitDirection::Horizontal => "-h",
            SplitDirection::Vertical => "-v",
        }
    }
}

/// Represents an agent running in a tmux pane
#[derive(Debug, Clone)]
pub struct AgentPane {
    pub pane_id: String,
    pub branch_name: String,
    pub agent_name: String,
    pub start_time: SystemTime,
    pub pid: u32,
    /// Whether the pane is in background (hidden from GWT window)
    pub is_background: bool,
    /// Window ID when pane is in background (for restoring)
    pub background_window: Option<String>,
}

impl AgentPane {
    /// Create a new AgentPane
    pub fn new(
        pane_id: String,
        branch_name: String,
        agent_name: String,
        start_time: SystemTime,
        pid: u32,
    ) -> Self {
        Self {
            pane_id,
            branch_name,
            agent_name,
            start_time,
            pid,
            is_background: false,
            background_window: None,
        }
    }

    /// Calculate uptime duration
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed().unwrap_or(Duration::from_secs(0))
    }

    /// Format uptime as hh:mm:ss
    pub fn uptime_string(&self) -> String {
        let duration = self.uptime();
        let secs = duration.as_secs();
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    }

    /// Check if termination confirmation is required
    /// (placeholder for future policy adjustments)
    pub fn requires_termination_confirmation(&self) -> bool {
        // Always require confirmation for running agents
        true
    }
}

/// Create a new pane in a session by splitting
///
/// # Arguments
/// * `session` - The session name
/// * `working_dir` - The working directory for the pane
/// * `command` - The command to run in the pane
///
/// # Returns
/// The pane ID of the newly created pane
pub fn create_pane(session: &str, working_dir: &str, command: &str) -> TmuxResult<String> {
    // Split the window horizontally and capture the pane ID
    let output = Command::new("tmux")
        .args([
            "split-window",
            "-h", // horizontal split
            "-t",
            session,
            "-c",
            working_dir,
            "-P", // print pane info
            "-F",
            "#{pane_id}",
            command,
        ])
        .output()
        .map_err(|e| TmuxError::PaneCreateFailed {
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::PaneCreateFailed {
            reason: stderr.to_string(),
        });
    }

    let pane_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(pane_id)
}

/// List all panes in a session (across all windows)
///
/// # Arguments
/// * `session` - The session name
///
/// # Returns
/// A vector of PaneInfo
pub fn list_panes(session: &str) -> TmuxResult<Vec<PaneInfo>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-s", // list all panes in session (across all windows)
            "-t",
            session,
            "-F",
            "#{pane_id}:#{pane_pid}:#{pane_current_command}:#{pane_current_path}",
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "list-panes".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "list-panes".to_string(),
            reason: stderr.to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_pane_list(&stdout))
}

/// List pane geometries in a target (window or session)
pub fn list_pane_geometries(target: &str) -> TmuxResult<Vec<PaneGeometry>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            target,
            "-F",
            "#{pane_id}:#{pane_left}:#{pane_top}:#{pane_width}:#{pane_height}",
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "list-panes".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "list-panes".to_string(),
            reason: stderr.to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_pane_geometry_list(&stdout))
}

/// Parse tmux list-panes output
/// Format: pane_id:pane_pid:current_command:current_path
pub fn parse_pane_list(output: &str) -> Vec<PaneInfo> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 3 {
                Some(PaneInfo {
                    pane_id: parts[0].to_string(),
                    pane_pid: parts[1].parse().unwrap_or(0),
                    current_command: parts[2].to_string(),
                    current_path: parts.get(3).map(|s| s.to_string()),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Parse tmux list-panes output for pane geometry
/// Format: pane_id:left:top:width:height
fn parse_pane_geometry_list(output: &str) -> Vec<PaneGeometry> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(5, ':').collect();
            if parts.len() != 5 {
                return None;
            }
            Some(PaneGeometry {
                pane_id: parts[0].to_string(),
                left: parts[1].parse().ok()?,
                top: parts[2].parse().ok()?,
                width: parts[3].parse().ok()?,
                height: parts[4].parse().ok()?,
            })
        })
        .collect()
}

/// Group panes by left coordinate (columns), ordered left-to-right and top-to-bottom
pub fn group_panes_by_left(panes: &[PaneGeometry]) -> Vec<PaneColumn> {
    let mut columns: std::collections::BTreeMap<u16, Vec<&PaneGeometry>> =
        std::collections::BTreeMap::new();

    for pane in panes {
        columns.entry(pane.left).or_default().push(pane);
    }

    columns
        .into_iter()
        .map(|(left, mut panes)| {
            panes.sort_by_key(|p| p.top);
            let width = panes.iter().map(|p| p.width).max().unwrap_or(0);
            let total_height = panes.iter().map(|p| p.height).sum();
            let pane_ids = panes.iter().map(|p| p.pane_id.clone()).collect();
            PaneColumn {
                left,
                width,
                pane_ids,
                total_height,
            }
        })
        .collect()
}

/// Compute equal split sizes that sum to total
pub fn compute_equal_splits(total: u16, parts: usize) -> Vec<u16> {
    if parts == 0 {
        return Vec::new();
    }
    let parts_u16 = parts as u16;
    let base = total / parts_u16;
    let remainder = total % parts_u16;
    let mut splits = vec![base; parts];
    for idx in 0..(remainder as usize) {
        splits[idx] += 1;
    }
    splits
}

/// Kill a specific pane
pub fn kill_pane(pane_id: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["kill-pane", "-t", pane_id])
        .output()
        .map_err(|e| TmuxError::PaneKillFailed {
            pane_id: pane_id.to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::PaneKillFailed {
            pane_id: pane_id.to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Select (focus) a specific pane
pub fn select_pane(pane_id: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "select-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(TmuxError::PaneNotFound {
            pane_id: pane_id.to_string(),
        });
    }

    Ok(())
}

/// Resize pane width (columns)
pub fn resize_pane_width(pane_id: &str, width: u16) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["resize-pane", "-t", pane_id, "-x", &width.to_string()])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "resize-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "resize-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Resize pane height (rows)
pub fn resize_pane_height(pane_id: &str, height: u16) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["resize-pane", "-t", pane_id, "-y", &height.to_string()])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "resize-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "resize-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Hide a pane by moving it to a separate background window
///
/// Uses `tmux break-pane` to move the pane to its own window without switching focus.
///
/// # Arguments
/// * `pane_id` - The pane ID to hide
/// * `window_name` - Name for the background window
///
/// # Returns
/// The window ID of the newly created background window
pub fn hide_pane(pane_id: &str, window_name: &str) -> TmuxResult<String> {
    // Get current session first
    let session_output = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "display-message".to_string(),
            reason: e.to_string(),
        })?;
    if !session_output.status.success() {
        let stderr = String::from_utf8_lossy(&session_output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "display-message".to_string(),
            reason: stderr.to_string(),
        });
    }

    let session_name = String::from_utf8_lossy(&session_output.stdout)
        .trim()
        .to_string();

    // Break pane into a new window (hidden, don't switch)
    let output = Command::new("tmux")
        .args([
            "break-pane",
            "-d", // don't switch to the new window
            "-s",
            pane_id,
            "-n",
            window_name,
            "-P",
            "-F",
            "#{window_id}",
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "break-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "break-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    // The break-pane command outputs the new window ID
    let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Return the full window reference (session:window)
    Ok(format!("{}:{}", session_name, window_id))
}

/// Enable tmux mouse support (global option)
fn enable_mouse() -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["set", "-g", "mouse", "on"])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "set".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "set".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Detach a pane into its own window without switching focus
pub fn break_pane(pane_id: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["break-pane", "-d", "-s", pane_id])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "break-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "break-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Show a hidden pane by joining it back to the GWT window
///
/// Uses `tmux join-pane` to move the pane from its background window back to the main window.
///
/// # Arguments
/// * `pane_id` - The pane ID to join back to the main window
/// * `target_pane_id` - The pane ID to join beside (usually the GWT pane)
///
/// # Returns
/// The new pane ID after joining
pub fn show_pane(pane_id: &str, target_pane_id: &str) -> TmuxResult<String> {
    // Ensure mouse mode is on when showing panes
    let _ = enable_mouse();

    // Join the pane back to the target pane
    let output = Command::new("tmux")
        .args([
            "join-pane",
            "-d", // don't switch focus
            "-h", // horizontal split (side by side)
            "-s",
            pane_id,
            "-t",
            target_pane_id,
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "join-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "join-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(pane_id.to_string())
}

/// Join a pane to a target with a split direction
pub fn join_pane_to_target(
    pane_id: &str,
    target_pane_id: &str,
    direction: SplitDirection,
) -> TmuxResult<String> {
    let split_flag = direction.tmux_flag();

    let output = Command::new("tmux")
        .args([
            "join-pane",
            "-d",
            split_flag,
            "-s",
            pane_id,
            "-t",
            target_pane_id,
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "join-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "join-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(pane_id.to_string())
}

/// Send keys to a pane (e.g., Ctrl-C for interrupt)
pub fn send_keys(pane_id: &str, keys: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["send-keys", "-t", pane_id, keys])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "send-keys".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "send-keys".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Check if exit confirmation is required based on running agents
pub fn requires_exit_confirmation(agents: &[AgentPane]) -> bool {
    !agents.is_empty()
}

/// Check for duplicate agent launch (same branch + same agent)
pub fn is_duplicate_launch(branch: &str, agent: &str, running: &[AgentPane]) -> bool {
    running
        .iter()
        .any(|p| p.branch_name == branch && p.agent_name == agent)
}

/// Signal type for terminating processes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermSignal {
    /// SIGTERM (graceful termination)
    Term,
    /// SIGKILL (forced termination)
    Kill,
}

impl TermSignal {
    /// Get the signal name for the kill command
    pub fn as_str(&self) -> &'static str {
        match self {
            TermSignal::Term => "TERM",
            TermSignal::Kill => "KILL",
        }
    }
}

/// Send a termination signal to a process
///
/// # Arguments
/// * `pid` - The process ID to signal
/// * `signal` - The signal type (TERM or KILL)
pub fn send_signal(pid: u32, signal: TermSignal) -> TmuxResult<()> {
    let output = Command::new("kill")
        .args([&format!("-{}", signal.as_str()), &pid.to_string()])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "kill".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Process may have already exited, which is fine
        if !stderr.contains("No such process") {
            return Err(TmuxError::CommandFailed {
                command: "kill".to_string(),
                reason: stderr.to_string(),
            });
        }
    }

    Ok(())
}

/// Gracefully terminate an agent pane
///
/// Sends SIGTERM first, allowing the agent to clean up.
/// If the process doesn't exit within timeout, caller should escalate to SIGKILL.
pub fn terminate_agent(pane: &AgentPane) -> TmuxResult<()> {
    // First try sending Ctrl-C via tmux
    let _ = send_keys(&pane.pane_id, "C-c");

    // Then send SIGTERM to the process
    send_signal(pane.pid, TermSignal::Term)
}

/// Forcefully kill an agent pane
///
/// Sends SIGKILL for immediate termination and then kills the tmux pane.
pub fn force_kill_agent(pane: &AgentPane) -> TmuxResult<()> {
    // Send SIGKILL to the process
    let _ = send_signal(pane.pid, TermSignal::Kill);

    // Kill the tmux pane
    kill_pane(&pane.pane_id)
}

/// Check if a process is still running
pub fn is_process_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Detect orphaned agent panes and match them to worktrees
///
/// This function is used to reconnect to existing agent panes when gwt restarts.
/// It scans all panes in the session and matches their working directory to worktree paths.
///
/// # Arguments
/// * `session` - The tmux session name
/// * `worktrees` - A slice of (branch_name, worktree_path) tuples
/// * `gwt_pane_id` - The pane ID of the gwt TUI (to exclude from detection)
///
/// # Returns
/// A vector of AgentPane for each pane whose current_path matches a worktree path
pub fn detect_orphaned_panes(
    session: &str,
    worktrees: &[(String, std::path::PathBuf)],
    gwt_pane_id: Option<&str>,
) -> TmuxResult<Vec<AgentPane>> {
    fn normalize_path(path: &str) -> &str {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            "/"
        } else {
            trimmed
        }
    }

    let panes = list_panes(session)?;
    let mut agents = Vec::new();

    for pane in panes {
        // Skip the gwt pane itself
        if let Some(gwt_id) = gwt_pane_id {
            if pane.pane_id == gwt_id {
                continue;
            }
        }

        if let Some(current_path) = &pane.current_path {
            // Check if current_path matches any worktree path
            let current_norm = normalize_path(current_path);
            for (branch_name, worktree_path) in worktrees {
                let worktree_str = worktree_path.to_string_lossy();
                let worktree_norm = normalize_path(worktree_str.as_ref());
                if current_norm == worktree_norm {
                    // Found a match - create AgentPane
                    // Detect agent name from the command
                    if let Some(agent_name) = detect_agent_name(&pane.current_command) {
                        agents.push(AgentPane::new(
                            pane.pane_id.clone(),
                            branch_name.clone(),
                            agent_name,
                            SystemTime::now(), // We don't know the actual start time
                            pane.pane_pid,
                        ));
                    }
                    break;
                }
            }
        }
    }

    Ok(agents)
}

/// Detect agent name from the pane's current command
fn detect_agent_name(command: &str) -> Option<String> {
    let cmd_lower = command.to_lowercase();

    // Known agent commands
    if cmd_lower.contains("claude") {
        Some("claude".to_string())
    } else if cmd_lower.contains("codex") {
        Some("codex".to_string())
    } else if cmd_lower.contains("aider") {
        Some("aider".to_string())
    } else if cmd_lower.contains("cursor") {
        Some("cursor".to_string())
    } else if cmd_lower.contains("cline") {
        Some("cline".to_string())
    } else if cmd_lower.contains("copilot") {
        Some("copilot".to_string())
    } else if cmd_lower.contains("gemini") {
        Some("gemini".to_string())
    } else if cmd_lower.contains("gpt") {
        Some("gpt".to_string())
    } else {
        None // Not a recognized agent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_pane_creation() {
        let pane = AgentPane::new(
            "1".to_string(),
            "feature/test".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        );
        assert_eq!(pane.branch_name, "feature/test");
        assert_eq!(pane.agent_name, "claude");
        assert_eq!(pane.pane_id, "1");
        assert_eq!(pane.pid, 12345);
    }

    #[test]
    fn test_pane_uptime_calculation() {
        let start = SystemTime::now() - Duration::from_secs(3661);
        let pane = AgentPane::new(
            "1".to_string(),
            "main".to_string(),
            "codex".to_string(),
            start,
            12345,
        );
        let uptime = pane.uptime_string();
        // 3661 seconds = 1h 1m 1s = "01:01:01"
        assert_eq!(uptime, "01:01:01");
    }

    #[test]
    fn test_pane_uptime_string_seconds() {
        let start = SystemTime::now() - Duration::from_secs(45);
        let pane = AgentPane::new(
            "1".to_string(),
            "main".to_string(),
            "claude".to_string(),
            start,
            12345,
        );
        let uptime = pane.uptime_string();
        // 45 seconds = "00:00:45"
        assert_eq!(uptime, "00:00:45");
    }

    #[test]
    fn test_pane_uptime_string_minutes() {
        let start = SystemTime::now() - Duration::from_secs(125);
        let pane = AgentPane::new(
            "1".to_string(),
            "main".to_string(),
            "claude".to_string(),
            start,
            12345,
        );
        let uptime = pane.uptime_string();
        // 125 seconds = 2m 5s = "00:02:05"
        assert_eq!(uptime, "00:02:05");
    }

    #[test]
    fn test_parse_pane_list_output() {
        let output =
            "0:12345:bash:/home/user\n1:12346:claude:/home/user/project\n2:12347:codex:/tmp";
        let panes = parse_pane_list(output);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].pane_id, "0");
        assert_eq!(panes[0].pane_pid, 12345);
        assert_eq!(panes[0].current_command, "bash");
        assert_eq!(panes[0].current_path, Some("/home/user".to_string()));
        assert_eq!(panes[1].pane_id, "1");
        assert_eq!(panes[1].current_command, "claude");
        assert_eq!(
            panes[1].current_path,
            Some("/home/user/project".to_string())
        );
    }

    #[test]
    fn test_parse_pane_list_without_path() {
        // Legacy format without current_path
        let output = "0:12345:bash\n1:12346:claude";
        let panes = parse_pane_list(output);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].current_path, None);
        assert_eq!(panes[1].current_path, None);
    }

    #[test]
    fn test_parse_pane_list_empty() {
        let panes = parse_pane_list("");
        assert!(panes.is_empty());
    }

    #[test]
    fn test_parse_pane_geometry_list() {
        let output = "%1:0:0:80:24\n%2:80:0:80:12\n%3:80:12:80:12";
        let panes = parse_pane_geometry_list(output);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].pane_id, "%1");
        assert_eq!(panes[0].left, 0);
        assert_eq!(panes[1].top, 0);
        assert_eq!(panes[2].height, 12);
    }

    #[test]
    fn test_group_panes_by_left() {
        let panes = vec![
            PaneGeometry {
                pane_id: "%1".to_string(),
                left: 0,
                top: 0,
                width: 80,
                height: 24,
            },
            PaneGeometry {
                pane_id: "%2".to_string(),
                left: 80,
                top: 12,
                width: 80,
                height: 12,
            },
            PaneGeometry {
                pane_id: "%3".to_string(),
                left: 80,
                top: 0,
                width: 80,
                height: 12,
            },
        ];
        let columns = group_panes_by_left(&panes);
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].left, 0);
        assert_eq!(columns[1].left, 80);
        assert_eq!(
            columns[1].pane_ids,
            vec!["%3".to_string(), "%2".to_string()]
        );
        assert_eq!(columns[1].total_height, 24);
    }

    #[test]
    fn test_compute_equal_splits() {
        assert_eq!(compute_equal_splits(9, 3), vec![3, 3, 3]);
        assert_eq!(compute_equal_splits(10, 3), vec![4, 3, 3]);
        assert_eq!(compute_equal_splits(5, 1), vec![5]);
        assert!(compute_equal_splits(0, 0).is_empty());
    }

    #[test]
    fn test_requires_exit_confirmation_with_agents() {
        let agents = vec![AgentPane::new(
            "1".to_string(),
            "feature/a".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        )];
        assert!(requires_exit_confirmation(&agents));
    }

    #[test]
    fn test_requires_exit_confirmation_without_agents() {
        let agents: Vec<AgentPane> = vec![];
        assert!(!requires_exit_confirmation(&agents));
    }

    #[test]
    fn test_is_duplicate_launch() {
        let running = vec![AgentPane::new(
            "1".to_string(),
            "feature/a".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        )];
        assert!(is_duplicate_launch("feature/a", "claude", &running));
    }

    #[test]
    fn test_no_duplicate_different_branch() {
        let running = vec![AgentPane::new(
            "1".to_string(),
            "feature/a".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        )];
        assert!(!is_duplicate_launch("feature/b", "claude", &running));
    }

    #[test]
    fn test_no_duplicate_different_agent() {
        let running = vec![AgentPane::new(
            "1".to_string(),
            "feature/a".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        )];
        assert!(!is_duplicate_launch("feature/a", "codex", &running));
    }

    #[test]
    fn test_requires_termination_confirmation() {
        let pane = AgentPane::new(
            "1".to_string(),
            "feature/test".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        );
        assert!(pane.requires_termination_confirmation());
    }

    #[test]
    fn test_term_signal_as_str() {
        assert_eq!(TermSignal::Term.as_str(), "TERM");
        assert_eq!(TermSignal::Kill.as_str(), "KILL");
    }

    #[test]
    fn test_is_process_running_nonexistent() {
        // PID 0 is the kernel, should not be signalable by regular users
        // PID 99999999 shouldn't exist
        assert!(!is_process_running(99999999));
    }

    #[test]
    fn test_is_process_running_self() {
        // Current process should be running
        let pid = std::process::id();
        assert!(is_process_running(pid));
    }

    #[test]
    fn test_agent_pane_default_not_background() {
        let pane = AgentPane::new(
            "1".to_string(),
            "feature/test".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        );
        assert!(!pane.is_background);
        assert!(pane.background_window.is_none());
    }

    #[test]
    fn test_agent_pane_background_state() {
        let mut pane = AgentPane::new(
            "1".to_string(),
            "feature/test".to_string(),
            "claude".to_string(),
            SystemTime::now(),
            12345,
        );
        // Simulate hiding the pane
        pane.is_background = true;
        pane.background_window = Some("session:@1".to_string());
        assert!(pane.is_background);
        assert_eq!(pane.background_window, Some("session:@1".to_string()));

        // Simulate showing the pane
        pane.is_background = false;
        pane.background_window = None;
        assert!(!pane.is_background);
        assert!(pane.background_window.is_none());
    }

    #[test]
    fn test_detect_agent_name_known_agents() {
        assert_eq!(detect_agent_name("claude"), Some("claude".to_string()));
        assert_eq!(detect_agent_name("Claude"), Some("claude".to_string()));
        assert_eq!(detect_agent_name("codex"), Some("codex".to_string()));
        assert_eq!(detect_agent_name("aider"), Some("aider".to_string()));
        assert_eq!(detect_agent_name("cursor"), Some("cursor".to_string()));
        assert_eq!(detect_agent_name("cline"), Some("cline".to_string()));
        assert_eq!(detect_agent_name("copilot"), Some("copilot".to_string()));
        assert_eq!(detect_agent_name("gemini"), Some("gemini".to_string()));
        assert_eq!(detect_agent_name("gpt"), Some("gpt".to_string()));
    }

    #[test]
    fn test_detect_agent_name_unknown() {
        assert_eq!(detect_agent_name("bash"), None);
        assert_eq!(detect_agent_name("vim"), None);
        assert_eq!(detect_agent_name("zsh"), None);
    }

    #[test]
    fn test_detect_agent_name_case_insensitive() {
        assert_eq!(detect_agent_name("CLAUDE"), Some("claude".to_string()));
        assert_eq!(detect_agent_name("Claude"), Some("claude".to_string()));
        assert_eq!(detect_agent_name("CODEX"), Some("codex".to_string()));
    }
}
