//! Terminal view widget: renders VT100 parser screen into ratatui buffer
//!
//! Phase 2: will use renderer.rs for full VT100 → ratatui conversion.

use ratatui::prelude::*;

/// Render a VT100 parser screen into the given buffer area.
pub fn render(_buf: &mut Buffer, _area: Rect, _parser: Option<&vt100::Parser>) {
    // Phase 2: delegate to renderer module for VT100 cell conversion
}
