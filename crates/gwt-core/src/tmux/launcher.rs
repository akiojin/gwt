//! tmux agent launcher
//!
//! Provides functionality to launch coding agents in tmux panes.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use super::error::{TmuxError, TmuxResult};
use super::pane::{select_pane, AgentPane};

/// Configuration for launching an agent in a tmux pane
#[derive(Debug, Clone)]
pub struct TmuxLaunchConfig {
    /// The tmux session name
    pub session: String,
    /// Working directory for the agent
    pub working_dir: String,
    /// The command to execute
    pub command: String,
    /// Command arguments
    pub args: Vec<String>,
    /// Environment variables to set
    pub env: HashMap<String, String>,
    /// Environment variables to remove
    pub env_remove: Vec<String>,
    /// Branch name for tracking
    pub branch_name: String,
    /// Agent name for tracking
    pub agent_name: String,
    /// Whether to focus the new pane after creation
    pub focus: bool,
}

/// Result of launching an agent in tmux
#[derive(Debug, Clone)]
pub struct TmuxLaunchResult {
    /// The pane ID of the newly created pane
    pub pane_id: String,
    /// The process ID of the agent
    pub pid: u32,
    /// Agent pane info for tracking
    pub agent_pane: AgentPane,
}

/// Launch an agent in a new tmux pane
///
/// Creates a new pane by splitting the current window, sets up the environment,
/// and executes the agent command.
pub fn launch_agent_in_pane(config: &TmuxLaunchConfig) -> TmuxResult<TmuxLaunchResult> {
    // Build the command string with environment setup
    let env_setup = build_env_setup(&config.env, &config.env_remove);
    let full_command = if env_setup.is_empty() {
        format!("{} {}", config.command, config.args.join(" "))
    } else {
        format!(
            "{}; {} {}",
            env_setup,
            config.command,
            config.args.join(" ")
        )
    };

    // Create a new pane with split-window
    let output = Command::new("tmux")
        .args([
            "split-window",
            "-h", // horizontal split
            "-t",
            &config.session,
            "-c",
            &config.working_dir,
            "-P", // print pane info
            "-F",
            "#{pane_id}:#{pane_pid}",
            "sh",
            "-c",
            &full_command,
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (pane_id, pid) = parse_pane_info(&stdout)?;

    let agent_pane = AgentPane::new(
        pane_id.clone(),
        config.branch_name.clone(),
        config.agent_name.clone(),
        SystemTime::now(),
        pid,
    );

    // Focus the new pane if requested
    if config.focus {
        select_pane(&pane_id)?;
    }

    Ok(TmuxLaunchResult {
        pane_id,
        pid,
        agent_pane,
    })
}

/// Build environment setup shell commands
fn build_env_setup(env: &HashMap<String, String>, env_remove: &[String]) -> String {
    let mut commands = Vec::new();

    // Unset environment variables
    for var in env_remove {
        commands.push(format!("unset {}", var));
    }

    // Export environment variables
    for (key, value) in env {
        // Escape single quotes in value
        let escaped_value = value.replace('\'', "'\\''");
        commands.push(format!("export {}='{}'", key, escaped_value));
    }

    commands.join("; ")
}

/// Parse pane info from tmux output (format: "pane_id:pane_pid")
fn parse_pane_info(output: &str) -> TmuxResult<(String, u32)> {
    let trimmed = output.trim();
    let parts: Vec<&str> = trimmed.splitn(2, ':').collect();

    if parts.len() != 2 {
        return Err(TmuxError::PaneCreateFailed {
            reason: format!("Invalid pane info format: {}", output),
        });
    }

    let pane_id = parts[0].to_string();
    let pid = parts[1].parse().map_err(|_| TmuxError::PaneCreateFailed {
        reason: format!("Invalid PID: {}", parts[1]),
    })?;

    Ok((pane_id, pid))
}

/// Build an agent command string from parameters
pub fn build_agent_command(
    agent_name: &str,
    model: Option<&str>,
    version: Option<&str>,
    execution_mode: &str,
    session_id: Option<&str>,
    skip_permissions: bool,
    env: &[(String, String)],
) -> String {
    let mut parts = Vec::new();

    // Add environment variable exports
    for (key, value) in env {
        let escaped_value = value.replace('\'', "'\\''");
        parts.push(format!("export {}='{}'", key, escaped_value));
    }

    // Build the agent command
    let mut cmd = agent_name.to_string();

    if let Some(m) = model {
        if !m.is_empty() {
            cmd.push_str(&format!(" --model {}", m));
        }
    }

    if execution_mode == "continue" {
        cmd.push_str(" --continue");
        if let Some(sid) = session_id {
            cmd.push_str(&format!(" {}", sid));
        }
    } else if execution_mode == "resume" {
        cmd.push_str(" --resume");
        if let Some(sid) = session_id {
            cmd.push_str(&format!(" {}", sid));
        }
    }

    if skip_permissions {
        cmd.push_str(" --dangerously-skip-permissions");
    }

    // Version is typically not used as CLI arg but could be for specific agents
    let _ = version; // Suppress unused warning

    if parts.is_empty() {
        cmd
    } else {
        parts.push(cmd);
        parts.join("; ")
    }
}

/// Launch a command in a new tmux pane (simplified API)
///
/// Creates a new pane below the current one and executes the command.
/// The command is executed with `exec` to replace the shell process,
/// ensuring the pane stays open while the agent runs.
///
/// Returns the pane ID of the newly created pane.
pub fn launch_in_pane(session: &str, working_dir: &str, command: &str) -> TmuxResult<String> {
    // Use exec to replace shell with the agent process
    // This ensures the pane stays open while the agent is running
    let exec_command = format!("exec {}", command);

    let output = Command::new("tmux")
        .args([
            "split-window",
            "-v", // vertical split (below current pane)
            "-d", // don't switch to new pane (keep focus on gwt)
            "-t",
            session,
            "-c",
            working_dir,
            "-P", // print pane info
            "-F",
            "#{pane_id}",
            "sh",
            "-c",
            &exec_command,
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

/// Launch a command in a new tmux pane beside an existing pane (horizontal split)
///
/// Creates a new pane to the right of the target pane and executes the command.
/// Used for adding additional agents beside existing agent panes.
///
/// Returns the pane ID of the newly created pane.
pub fn launch_in_pane_beside(
    target_pane: &str,
    working_dir: &str,
    command: &str,
) -> TmuxResult<String> {
    // Use exec to replace shell with the agent process
    let exec_command = format!("exec {}", command);

    let output = Command::new("tmux")
        .args([
            "split-window",
            "-h", // horizontal split (beside target pane)
            "-d", // don't switch to new pane (keep focus on gwt)
            "-t",
            target_pane,
            "-c",
            working_dir,
            "-P", // print pane info
            "-F",
            "#{pane_id}",
            "sh",
            "-c",
            &exec_command,
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

/// Check if a directory exists and is valid for agent execution
pub fn validate_working_dir(path: &Path) -> TmuxResult<()> {
    if !path.exists() {
        return Err(TmuxError::CommandFailed {
            command: "validate_working_dir".to_string(),
            reason: format!("Directory does not exist: {}", path.display()),
        });
    }

    if !path.is_dir() {
        return Err(TmuxError::CommandFailed {
            command: "validate_working_dir".to_string(),
            reason: format!("Path is not a directory: {}", path.display()),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_env_setup_empty() {
        let env = HashMap::new();
        let env_remove = vec![];
        let result = build_env_setup(&env, &env_remove);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_env_setup_with_vars() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        env.insert("BAZ".to_string(), "qux".to_string());
        let env_remove = vec![];
        let result = build_env_setup(&env, &env_remove);
        assert!(result.contains("export FOO='bar'"));
        assert!(result.contains("export BAZ='qux'"));
    }

    #[test]
    fn test_build_env_setup_with_unset() {
        let env = HashMap::new();
        let env_remove = vec!["OLD_VAR".to_string()];
        let result = build_env_setup(&env, &env_remove);
        assert_eq!(result, "unset OLD_VAR");
    }

    #[test]
    fn test_build_env_setup_escape_quotes() {
        let mut env = HashMap::new();
        env.insert("MSG".to_string(), "it's a test".to_string());
        let env_remove = vec![];
        let result = build_env_setup(&env, &env_remove);
        assert!(result.contains("'it'\\''s a test'"));
    }

    #[test]
    fn test_parse_pane_info_valid() {
        let (pane_id, pid) = parse_pane_info("%5:12345\n").unwrap();
        assert_eq!(pane_id, "%5");
        assert_eq!(pid, 12345);
    }

    #[test]
    fn test_parse_pane_info_invalid() {
        let result = parse_pane_info("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_working_dir_nonexistent() {
        let result = validate_working_dir(Path::new("/nonexistent/path/12345"));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_working_dir_valid() {
        let result = validate_working_dir(Path::new("/tmp"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_tmux_launch_config_creation() {
        let config = TmuxLaunchConfig {
            session: "gwt-test".to_string(),
            working_dir: "/tmp".to_string(),
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            env: HashMap::new(),
            env_remove: vec![],
            branch_name: "feature/test".to_string(),
            agent_name: "claude".to_string(),
            focus: true,
        };
        assert_eq!(config.session, "gwt-test");
        assert_eq!(config.branch_name, "feature/test");
    }
}
