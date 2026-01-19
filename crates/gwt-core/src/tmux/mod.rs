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
    check_tmux_installed, get_current_pane_id, get_current_session, get_tmux_version,
    is_inside_tmux, TmuxVersion,
};
pub use error::{TmuxError, TmuxResult};
pub use keybind::{focus_gwt_pane, remove_ctrl_g_keybind, setup_ctrl_g_keybind, GWT_PANE_INDEX};
pub use launcher::{
    build_agent_command, launch_agent_in_pane, launch_in_pane, launch_in_pane_below,
    launch_in_pane_beside, TmuxLaunchConfig, TmuxLaunchResult,
};
pub use logging::{start_logging, stop_logging, LogConfig};
pub use naming::generate_session_name;
pub use pane::{
    break_pane, compute_equal_splits, detect_orphaned_panes, force_kill_agent, group_panes_by_left,
    hide_pane, is_process_running, join_pane_to_target, kill_pane, list_pane_geometries,
    resize_pane_height, resize_pane_width, send_signal, show_pane, terminate_agent, AgentPane,
    PaneColumn, PaneGeometry, PaneInfo, SplitDirection, TermSignal,
};
pub use poller::{AgentRegistry, PanePoller, PollMessage, PollerConfig};
