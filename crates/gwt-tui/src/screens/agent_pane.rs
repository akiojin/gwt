//! Agent/Shell pane screen
//!
//! Renders VT100 terminal output for an agent or shell session.

use ratatui::prelude::*;

/// Render a terminal pane using the VT100 parser screen.
pub fn render(buf: &mut Buffer, area: Rect, parser: Option<&vt100::Parser>) {
    if let Some(parser) = parser {
        crate::renderer::render_vt100_screen(buf, area, parser.screen());
    } else {
        // No parser yet — show placeholder
        let text = ratatui::widgets::Paragraph::new("Starting...")
            .alignment(Alignment::Center);
        ratatui::widgets::Widget::render(text, area, buf);
    }
}
