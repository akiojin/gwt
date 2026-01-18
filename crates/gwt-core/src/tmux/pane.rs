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

    /// Format uptime as a human-readable string
    pub fn uptime_string(&self) -> String {
        let duration = self.uptime();
        let secs = duration.as_secs();

        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }

    /// Check if termination confirmation is required
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

/// List all panes in a session
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
            "-t",
            session,
            "-F",
            "#{pane_id}:#{pane_pid}:#{pane_current_command}",
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

/// Parse tmux list-panes output
pub fn parse_pane_list(output: &str) -> Vec<PaneInfo> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() >= 3 {
                Some(PaneInfo {
                    pane_id: parts[0].to_string(),
                    pane_pid: parts[1].parse().unwrap_or(0),
                    current_command: parts[2].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
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

/// Show a hidden pane by joining it back to the GWT window
///
/// Uses `tmux join-pane` to move the pane from its background window back to the main window.
///
/// # Arguments
/// * `background_window` - The background window identifier (session:window_id)
/// * `target_pane_id` - The pane ID to join beside (usually the GWT pane)
///
/// # Returns
/// The new pane ID after joining
pub fn show_pane(background_window: &str, target_pane_id: &str) -> TmuxResult<String> {
    // Join the pane from the background window to the target pane
    let output = Command::new("tmux")
        .args([
            "join-pane",
            "-d", // don't switch focus
            "-h", // horizontal split (side by side)
            "-s",
            background_window,
            "-t",
            target_pane_id,
            "-P",
            "-F",
            "#{pane_id}",
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

    let new_pane_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(new_pane_id)
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
        assert!(uptime.contains("1h"));
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
        assert!(uptime.ends_with("s"));
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
        assert!(uptime.contains("m"));
    }

    #[test]
    fn test_parse_pane_list_output() {
        let output = "0:12345:bash\n1:12346:claude\n2:12347:codex";
        let panes = parse_pane_list(output);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].pane_id, "0");
        assert_eq!(panes[0].pane_pid, 12345);
        assert_eq!(panes[0].current_command, "bash");
        assert_eq!(panes[1].pane_id, "1");
        assert_eq!(panes[1].current_command, "claude");
    }

    #[test]
    fn test_parse_pane_list_empty() {
        let panes = parse_pane_list("");
        assert!(panes.is_empty());
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
}
