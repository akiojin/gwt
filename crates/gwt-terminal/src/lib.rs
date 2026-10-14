//! gwt-terminal: PTY management and vt100 terminal emulation.
//!
//! This crate provides the terminal subsystem for gwt:
//! - `PtyHandle` — cross-platform PTY spawn, I/O, resize, kill
//! - `Pane` — integrates PTY + vt100 parser (history lives in the parser's bounded scrollback)
//! - `PaneManager` — manages multiple panes with spawn/close/resize

pub mod manager;
pub mod pane;
pub mod pty;

#[cfg(test)]
pub(crate) mod test_util;

/// Issue #4234: the lib test binary counts live heap bytes per thread so
/// retention regressions are measured, not inferred.
#[cfg(test)]
#[global_allocator]
static COUNTING_ALLOCATOR: test_util::CountingAllocator = test_util::CountingAllocator;

pub use manager::PaneManager;
pub use pane::{Pane, PaneExit, PaneStatus, PendingPane, SNAPSHOT_SCROLLBACK_REPLAY_LIMIT};
pub use pty::{PendingPty, PtyHandle};
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
