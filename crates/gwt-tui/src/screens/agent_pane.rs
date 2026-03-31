//! Agent pane screen (Phase 2 stub)
//!
//! Renders VT100 terminal output for an agent or shell session.

use ratatui::prelude::*;

/// Render a terminal pane using the VT100 parser screen.
pub fn render(_buf: &mut Buffer, _area: Rect, _parser: Option<&vt100::Parser>) {
    // Phase 2: use renderer.rs to convert vt100 screen to ratatui cells
}
