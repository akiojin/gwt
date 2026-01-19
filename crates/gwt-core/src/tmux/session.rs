//! tmux session management
//!
//! Provides functions to create, destroy, and list tmux sessions.

use std::process::Command;

use super::error::{TmuxError, TmuxResult};

/// Create a new detached tmux session
///
/// # Arguments
/// * `name` - The session name
///
/// # Returns
/// Ok(()) on success, or TmuxError on failure
pub fn create_session(name: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["new-session", "-d", "-s", name])
        .output()
        .map_err(|e| TmuxError::SessionCreateFailed {
            name: name.to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("duplicate session") {
            return Err(TmuxError::SessionAlreadyExists {
                name: name.to_string(),
            });
        }
        return Err(TmuxError::SessionCreateFailed {
            name: name.to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Create a new detached tmux session with initial command
///
/// # Arguments
/// * `name` - The session name
/// * `working_dir` - The working directory for the session
/// * `command` - Optional command to run in the session
pub fn create_session_with_command(
    name: &str,
    working_dir: &str,
    command: Option<&[&str]>,
) -> TmuxResult<()> {
    let mut args = vec!["new-session", "-d", "-s", name, "-c", working_dir];

    if let Some(cmd) = command {
        args.extend(cmd);
    }

    let output =
        Command::new("tmux")
            .args(&args)
            .output()
            .map_err(|e| TmuxError::SessionCreateFailed {
                name: name.to_string(),
                reason: e.to_string(),
            })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::SessionCreateFailed {
            name: name.to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Destroy (kill) a tmux session
///
/// # Arguments
/// * `name` - The session name to destroy
pub fn destroy_session(name: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["kill-session", "-t", name])
        .output()
        .map_err(|e| TmuxError::SessionDestroyFailed {
            name: name.to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("session not found") {
            return Err(TmuxError::SessionNotFound {
                name: name.to_string(),
            });
        }
        return Err(TmuxError::SessionDestroyFailed {
            name: name.to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// List all tmux sessions
///
/// # Returns
/// A vector of session names
pub fn list_sessions() -> TmuxResult<Vec<String>> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "list-sessions".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // No sessions is not an error
        if stderr.contains("no server running") || stderr.contains("no sessions") {
            return Ok(vec![]);
        }
        return Err(TmuxError::CommandFailed {
            command: "list-sessions".to_string(),
            reason: stderr.to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sessions: Vec<String> = stdout
        .lines()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    Ok(sessions)
}

/// Check if a session exists
pub fn session_exists(name: &str) -> TmuxResult<bool> {
    let sessions = list_sessions()?;
    Ok(sessions.contains(&name.to_string()))
}

/// Attach to an existing session
pub fn attach_session(name: &str) -> TmuxResult<()> {
    let status = Command::new("tmux")
        .args(["attach-session", "-t", name])
        .status()
        .map_err(|e| TmuxError::CommandFailed {
            command: "attach-session".to_string(),
            reason: e.to_string(),
        })?;

    if !status.success() {
        return Err(TmuxError::SessionNotFound {
            name: name.to_string(),
        });
    }

    Ok(())
}

/// Switch to a session (when already inside tmux)
pub fn switch_to_session(name: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["switch-client", "-t", name])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "switch-client".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "switch-client".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_list_sessions_format() {
        // This test verifies the parsing logic, not actual tmux execution
        let mock_output = "gwt-myrepo\ngwt-other-2\nsome-session";
        let sessions: Vec<String> = mock_output
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        assert_eq!(sessions.len(), 3);
        assert!(sessions.contains(&"gwt-myrepo".to_string()));
        assert!(sessions.contains(&"gwt-other-2".to_string()));
    }
}
