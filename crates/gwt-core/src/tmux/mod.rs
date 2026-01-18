//! tmux integration module for gwt
//!
//! This module provides tmux session and pane management functionality
//! for running multiple coding agents in parallel.

pub mod detector;
pub mod error;
pub mod naming;
pub mod pane;
pub mod session;

pub use detector::{check_tmux_installed, get_tmux_version, is_inside_tmux, TmuxVersion};
pub use error::{TmuxError, TmuxResult};
pub use naming::generate_session_name;
pub use pane::{AgentPane, PaneInfo};
pub use session::TmuxSession;
