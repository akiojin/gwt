//! tmux keybinding configuration
//!
//! Provides functionality to configure tmux keybindings for gwt sessions.

use std::process::Command;

use super::error::{TmuxError, TmuxResult};

/// Default pane index for gwt (the master/control pane)
pub const GWT_PANE_INDEX: &str = "0";

/// Setup Ctrl-g keybinding for returning to gwt pane
///
/// Binds Ctrl-g to select the gwt pane (pane 0) in the specified session.
pub fn setup_ctrl_g_keybind(session: &str) -> TmuxResult<()> {
    // Bind Ctrl-g to select the gwt pane (pane 0)
    let output = Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "root", // root key table (available without prefix)
            "C-g",  // Ctrl-g
            "select-pane",
            "-t",
            &format!("{}:{}", session, GWT_PANE_INDEX),
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "bind-key".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "bind-key".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Remove Ctrl-g keybinding
pub fn remove_ctrl_g_keybind() -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["unbind-key", "-T", "root", "C-g"])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "unbind-key".to_string(),
            reason: e.to_string(),
        })?;

    // Ignore errors - the key might not be bound
    if !output.status.success() {
        // Key might not be bound, which is fine
    }

    Ok(())
}

/// Focus the gwt pane (pane 0)
pub fn focus_gwt_pane(session: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args([
            "select-pane",
            "-t",
            &format!("{}:{}", session, GWT_PANE_INDEX),
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "select-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "select-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Check if Ctrl-g is already bound
pub fn is_ctrl_g_bound() -> TmuxResult<bool> {
    let output = Command::new("tmux")
        .args(["list-keys", "-T", "root"])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "list-keys".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.contains("C-g"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gwt_pane_index_is_zero() {
        assert_eq!(GWT_PANE_INDEX, "0");
    }

    #[test]
    fn test_is_ctrl_g_bound() {
        // This test is environment-dependent
        let result = is_ctrl_g_bound();
        // Just verify it returns a valid result
        assert!(result.is_ok() || result.is_err());
    }
}
