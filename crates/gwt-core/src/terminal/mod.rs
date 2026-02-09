pub mod error;
pub mod ipc;
pub mod manager;
pub mod pane;
pub mod pty;
pub mod runner;
pub mod scrollback;

use std::collections::HashMap;
use std::path::PathBuf;

pub use error::TerminalError;

/// Agent color representation for UI rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentColor {
    Green,
    Blue,
    Cyan,
    Red,
    Yellow,
    Magenta,
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

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
    pub agent_color: AgentColor,
    /// Environment variables to set.
    pub env_vars: HashMap<String, String>,
}
