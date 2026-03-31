//! Issues screen (Phase 2 stub)

use ratatui::prelude::*;

/// Messages specific to the Issues screen.
#[derive(Debug)]
pub enum IssuesMessage {
    Refresh,
}

/// Render the issues screen into the given area.
pub fn render(_buf: &mut Buffer, _area: Rect) {
    // Phase 2: issues rendering
}

/// Handle a key event in the issues screen.
pub fn handle_key(
    _key: &crossterm::event::KeyEvent,
) -> Option<IssuesMessage> {
    None
}
