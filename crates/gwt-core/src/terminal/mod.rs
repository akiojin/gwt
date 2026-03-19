pub mod error;
pub mod ipc;
pub mod manager;
pub mod osc;
pub mod pane;
pub mod pty;
pub mod runner;
pub mod scrollback;
pub mod shell;

use std::{collections::HashMap, path::PathBuf};

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
    /// Optional shell override (e.g. "powershell", "cmd", "wsl").
    pub terminal_shell: Option<String>,
    /// Whether this launch is interactive (e.g. spawn_shell).
    /// When true on Windows, the command is not wrapped with PowerShell.
    pub interactive: bool,
    /// Whether to force UTF-8 terminal initialization on Windows launch.
    pub windows_force_utf8: bool,
}
