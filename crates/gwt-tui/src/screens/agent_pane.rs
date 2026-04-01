//! Agent/Shell pane screen
//!
//! Renders VT100 terminal output for an agent or shell session.

use ratatui::prelude::*;

/// Render a terminal pane using the VT100 parser screen.
/// Returns the cursor position (x, y) relative to the frame if visible.
pub fn render(buf: &mut Buffer, area: Rect, parser: Option<&vt100::Parser>) -> Option<(u16, u16)> {
    if let Some(parser) = parser {
        let screen = parser.screen();
        crate::renderer::render_vt100_screen(buf, area, screen);

        // Return cursor position if visible
        if !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            let x = area.x + col;
            let y = area.y + row;
            if x < area.right() && y < area.bottom() {
                return Some((x, y));
            }
        }
        None
    } else {
        let text = ratatui::widgets::Paragraph::new("Starting...").alignment(Alignment::Center);
        ratatui::widgets::Widget::render(text, area, buf);
        None
    }
}
