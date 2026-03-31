//! Branches screen (Phase 2 stub)

use ratatui::prelude::*;

/// Messages specific to the Branches screen.
#[derive(Debug)]
pub enum BranchesMessage {
    Refresh,
}

/// Render the branches screen into the given area.
pub fn render(_buf: &mut Buffer, _area: Rect) {
    // Phase 2: branch list rendering
}

/// Handle a key event in the branches screen. Returns an optional message.
pub fn handle_key(
    _key: &crossterm::event::KeyEvent,
) -> Option<BranchesMessage> {
    None
}
