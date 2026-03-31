//! Settings screen (Phase 2 stub)

use ratatui::prelude::*;

/// Messages specific to the Settings screen.
#[derive(Debug)]
pub enum SettingsMessage {
    Refresh,
}

/// Render the settings screen into the given area.
pub fn render(_buf: &mut Buffer, _area: Rect) {
    // Phase 2: settings rendering
}

/// Handle a key event in the settings screen.
pub fn handle_key(_key: &crossterm::event::KeyEvent) -> Option<SettingsMessage> {
    None
}
