//! gwt-terminal: PTY management, vt100 terminal emulation, and scrollback.
//!
//! This crate provides the terminal subsystem for gwt:
//! - `PtyHandle` — cross-platform PTY spawn, I/O, resize, and kill
//! - `Pane` — integrates PTY + vt100 parser + scrollback
//! - `PaneManager` — manages multiple panes with spawn/close/resize
//! - `ScrollbackStorage` — memory-efficient ring buffer for terminal lines
//! - `runtime` — host terminal lifecycle + neutral crossterm event handling
//! - `protocol` — PTY input encoding helpers

pub mod manager;
pub mod pane;
pub mod protocol;
pub mod pty;
pub mod runtime;
pub mod scrollback;

#[cfg(test)]
pub(crate) mod test_util;

pub use manager::PaneManager;
pub use pane::{Pane, PaneStatus};
pub use pty::PtyHandle;
pub use scrollback::ScrollbackStorage;
use thiserror::Error;

/// Errors from the gwt-terminal subsystem.
#[derive(Error, Debug)]
pub enum TerminalError {
    #[error("PTY creation failed: {reason}")]
    PtyCreationFailed { reason: String },

    #[error("PTY I/O error: {details}")]
    PtyIoError { details: String },

    #[error("Pane not found: {id}")]
    PaneNotFound { id: String },
}
