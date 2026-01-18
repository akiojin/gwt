//! tmux integration module for gwt
//!
//! This module provides tmux session and pane management functionality
//! for running multiple coding agents in parallel.

pub mod detector;
pub mod error;
pub mod keybind;
pub mod launcher;
pub mod logging;
pub mod naming;
pub mod pane;
pub mod poller;
pub mod session;

pub use detector::{
    check_tmux_installed, get_current_session, get_tmux_version, is_inside_tmux, TmuxVersion,
};
pub use error::{TmuxError, TmuxResult};
pub use keybind::{focus_gwt_pane, remove_ctrl_g_keybind, setup_ctrl_g_keybind, GWT_PANE_INDEX};
pub use launcher::{
    build_agent_command, launch_agent_in_pane, launch_in_pane, TmuxLaunchConfig, TmuxLaunchResult,
};
pub use logging::{start_logging, stop_logging, LogConfig};
pub use naming::generate_session_name;
pub use pane::{
    force_kill_agent, is_process_running, send_signal, terminate_agent, AgentPane, PaneInfo,
    TermSignal,
};
pub use poller::{AgentRegistry, PanePoller, PollMessage, PollerConfig};
pub use session::TmuxSession;
