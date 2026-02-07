pub mod emulator;
pub mod error;
pub mod ipc;
pub mod manager;
pub mod pane;
pub mod pty;
pub mod renderer;
pub mod scrollback;

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::style::Color;

pub use error::TerminalError;

/// Configuration for launching an agent in the built-in terminal.
pub struct BuiltinLaunchConfig {
    /// The command to execute.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Working directory for the agent.
    pub working_dir: PathBuf,
    /// Branch name for tracking.
    pub branch_name: String,
    /// Agent name for display.
    pub agent_name: String,
    /// Agent color for tab/header.
    pub agent_color: Color,
    /// Environment variables to set.
    pub env_vars: HashMap<String, String>,
}
