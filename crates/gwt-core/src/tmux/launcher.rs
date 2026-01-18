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

/// Detect available UTF-8 locale on the system
///
/// Tries common UTF-8 locale names and returns the first available one.
/// Falls back to "C.UTF-8" if detection fails.
fn detect_utf8_locale() -> String {
    use std::process::Command;

    // Try to get available locales
    if let Ok(output) = Command::new("locale").arg("-a").output() {
        if output.status.success() {
            let locales = String::from_utf8_lossy(&output.stdout);
            // Check for common UTF-8 locales in order of preference
            let candidates = ["C.utf8", "C.UTF-8", "en_US.utf8", "en_US.UTF-8", "POSIX"];
            for candidate in candidates {
                if locales.lines().any(|l| l == candidate) {
                    return candidate.to_string();
                }
            }
        }
    }

    // Default fallback
    "C.UTF-8".to_string()
}

/// Get environment variables needed for proper Unicode display in tmux panes
///
/// Returns locale and terminal environment variables to ensure UTF-8 encoding
/// and proper Unicode rendering in new panes.
/// If no locale is configured in the current environment, uses C.UTF-8 as default.
fn get_locale_env_vars() -> Vec<(String, String)> {
    let mut vars = Vec::new();

    // Always inherit TERM for proper terminal capabilities (Unicode block elements, etc.)
    if let Ok(term) = std::env::var("TERM") {
        if !term.is_empty() {
            vars.push(("TERM".to_string(), term));
        }
    }

    // Check if any UTF-8 locale is already set
    let has_utf8_locale = ["LANG", "LC_ALL", "LC_CTYPE"]
        .iter()
        .any(|key| {
            std::env::var(key)
                .map(|v| v.to_uppercase().contains("UTF-8") || v.to_uppercase().contains("UTF8"))
                .unwrap_or(false)
        });

    if has_utf8_locale {
        // Inherit existing locale settings
        const LOCALE_KEYS: &[&str] = &[
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "LC_MESSAGES",
            "LC_COLLATE",
            "LC_TIME",
            "LC_NUMERIC",
            "LC_MONETARY",
        ];

        for key in LOCALE_KEYS {
            if let Ok(value) = std::env::var(key) {
                if !value.is_empty() {
                    vars.push((key.to_string(), value));
                }
            }
        }
    } else {
        // No UTF-8 locale configured, detect available UTF-8 locale
        // Try C.utf8 first (common on minimal Linux), then C.UTF-8
        let utf8_locale = detect_utf8_locale();
        vars.push(("LANG".to_string(), utf8_locale.clone()));
        vars.push(("LC_ALL".to_string(), utf8_locale));
    }

    vars
}

/// Build locale setup command string
#[cfg(test)]
fn build_locale_setup(locale_vars: &[(String, String)]) -> String {
    locale_vars
        .iter()
        .map(|(key, value)| {
            let escaped_value = value.replace('\'', "'\\''");
            format!("export {}='{}'", key, escaped_value)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Launch a command in a new tmux pane (simplified API)
///
/// Creates a new pane below the current one and executes the command.
/// The pane automatically closes when the command exits (FR-052).
/// Automatically inherits locale and terminal environment variables to prevent encoding issues.
///
/// Returns the pane ID of the newly created pane.
pub fn launch_in_pane(target_pane: &str, working_dir: &str, command: &str) -> TmuxResult<String> {
    // Build args with environment variables passed via -e option
    let env_vars = get_locale_env_vars();
    let mut args = vec![
        "split-window".to_string(),
        "-v".to_string(), // vertical split (below current pane)
        "-d".to_string(), // don't switch to new pane (keep focus on gwt)
        "-t".to_string(),
        target_pane.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
    ];

    // Add environment variables via -e option (tmux 3.0+)
    for (key, value) in &env_vars {
        args.push("-e".to_string());
        args.push(format!("{}={}", key, value));
    }

    // Add output format options
    args.push("-P".to_string()); // print pane info
    args.push("-F".to_string());
    args.push("#{pane_id}".to_string());

    // Create the pane (starts an interactive shell)
    let output = Command::new("tmux")
        .args(&args)
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

    // Set remain-on-exit off so pane auto-closes when command exits (FR-052)
    let _ = Command::new("tmux")
        .args(["set-option", "-t", &pane_id, "remain-on-exit", "off"])
        .output();

    // Send the command to the new pane via send-keys
    // This ensures stdin/stdout are properly connected to the terminal
    let command_with_exit = format!("{}; exit", command);
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &pane_id, &command_with_exit, "Enter"])
        .output();

    Ok(pane_id)
}

/// Launch a command in a new tmux pane beside an existing pane (horizontal split)
///
/// Creates a new pane to the right of the target pane and executes the command.
/// The pane automatically closes when the command exits (FR-052).
/// Automatically inherits locale and terminal environment variables to prevent encoding issues.
///
/// Returns the pane ID of the newly created pane.
pub fn launch_in_pane_beside(
    target_pane: &str,
    working_dir: &str,
    command: &str,
) -> TmuxResult<String> {
    // Build args with environment variables passed via -e option
    let env_vars = get_locale_env_vars();
    let mut args = vec![
        "split-window".to_string(),
        "-h".to_string(), // horizontal split (beside target pane)
        "-d".to_string(), // don't switch to new pane (keep focus on gwt)
        "-t".to_string(),
        target_pane.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
    ];

    // Add environment variables via -e option (tmux 3.0+)
    for (key, value) in &env_vars {
        args.push("-e".to_string());
        args.push(format!("{}={}", key, value));
    }

    // Add output format options
    args.push("-P".to_string()); // print pane info
    args.push("-F".to_string());
    args.push("#{pane_id}".to_string());

    // Create the pane (starts an interactive shell)
    let output = Command::new("tmux")
        .args(&args)
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

    // Set remain-on-exit off so pane auto-closes when command exits (FR-052)
    let _ = Command::new("tmux")
        .args(["set-option", "-t", &pane_id, "remain-on-exit", "off"])
        .output();

    // Send the command to the new pane via send-keys
    // This ensures stdin/stdout are properly connected to the terminal
    let command_with_exit = format!("{}; exit", command);
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &pane_id, &command_with_exit, "Enter"])
        .output();

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

    #[test]
    fn test_build_agent_command_basic() {
        let cmd = build_agent_command("claude", None, None, "normal", None, false, &[]);
        assert_eq!(cmd, "claude");
    }

    #[test]
    fn test_build_agent_command_with_model() {
        let cmd = build_agent_command("claude", Some("opus"), None, "normal", None, false, &[]);
        assert_eq!(cmd, "claude --model opus");
    }

    #[test]
    fn test_build_agent_command_continue_mode() {
        let cmd = build_agent_command("claude", None, None, "continue", None, false, &[]);
        assert_eq!(cmd, "claude --continue");
    }

    #[test]
    fn test_build_agent_command_continue_with_session() {
        let cmd = build_agent_command("claude", None, None, "continue", Some("abc123"), false, &[]);
        assert_eq!(cmd, "claude --continue abc123");
    }

    #[test]
    fn test_build_agent_command_resume_mode() {
        let cmd = build_agent_command("claude", None, None, "resume", Some("xyz789"), false, &[]);
        assert_eq!(cmd, "claude --resume xyz789");
    }

    #[test]
    fn test_build_agent_command_skip_permissions() {
        let cmd = build_agent_command("claude", None, None, "normal", None, true, &[]);
        assert_eq!(cmd, "claude --dangerously-skip-permissions");
    }

    #[test]
    fn test_build_agent_command_with_env() {
        let env = vec![("API_KEY".to_string(), "secret".to_string())];
        let cmd = build_agent_command("claude", None, None, "normal", None, false, &env);
        assert!(cmd.contains("export API_KEY='secret'"));
        assert!(cmd.contains("claude"));
    }

    #[test]
    fn test_build_agent_command_full() {
        let env = vec![("FOO".to_string(), "bar".to_string())];
        let cmd = build_agent_command(
            "claude",
            Some("sonnet"),
            Some("1.0.0"),
            "continue",
            Some("sess123"),
            true,
            &env,
        );
        assert!(cmd.contains("export FOO='bar'"));
        assert!(cmd.contains("claude"));
        assert!(cmd.contains("--model sonnet"));
        assert!(cmd.contains("--continue sess123"));
        assert!(cmd.contains("--dangerously-skip-permissions"));
    }

    #[test]
    fn test_build_agent_command_empty_model() {
        // Empty model should not add --model flag
        let cmd = build_agent_command("claude", Some(""), None, "normal", None, false, &[]);
        assert_eq!(cmd, "claude");
        assert!(!cmd.contains("--model"));
    }

    #[test]
    fn test_build_locale_setup_empty() {
        let result = build_locale_setup(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_locale_setup_with_vars() {
        let vars = vec![
            ("LANG".to_string(), "en_US.UTF-8".to_string()),
            ("LC_ALL".to_string(), "en_US.UTF-8".to_string()),
        ];
        let result = build_locale_setup(&vars);
        assert!(result.contains("export LANG='en_US.UTF-8'"));
        assert!(result.contains("export LC_ALL='en_US.UTF-8'"));
    }

    #[test]
    fn test_build_locale_setup_escape_quotes() {
        let vars = vec![("LANG".to_string(), "it's".to_string())];
        let result = build_locale_setup(&vars);
        assert!(result.contains("'it'\\''s'"));
    }

    #[test]
    fn test_get_locale_env_vars() {
        // This test verifies the function doesn't panic and returns a valid structure
        let vars = get_locale_env_vars();
        // All returned keys should be valid environment variable keys
        let valid_keys = [
            "TERM",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "LC_MESSAGES",
            "LC_COLLATE",
            "LC_TIME",
            "LC_NUMERIC",
            "LC_MONETARY",
        ];
        for (key, _) in &vars {
            assert!(
                valid_keys.contains(&key.as_str()),
                "Unexpected key: {}",
                key
            );
        }
    }
}
