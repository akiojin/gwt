//! Logs screen (Phase 2 stub)

use ratatui::prelude::*;

/// Messages specific to the Logs screen.
#[derive(Debug)]
pub enum LogsMessage {
    Refresh,
}

/// Render the logs screen into the given area.
pub fn render(_buf: &mut Buffer, _area: Rect) {
    // Phase 2: logs rendering
}

/// Handle a key event in the logs screen.
pub fn handle_key(
    _key: &crossterm::event::KeyEvent,
) -> Option<LogsMessage> {
    None
}
