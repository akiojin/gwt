//! Terminal view widget — renders a vt100 screen buffer.

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use crate::renderer;

/// Widget that renders a vt100 screen into a ratatui area.
pub struct TerminalView<'a> {
    screen: &'a vt100::Screen,
}

impl<'a> TerminalView<'a> {
    pub fn new(screen: &'a vt100::Screen) -> Self {
        Self { screen }
    }
}

impl Widget for TerminalView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        renderer::render_vt_screen(self.screen, buf, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_view_renders_without_panic() {
        let parser = vt100::Parser::new(4, 10, 0);
        let view = TerminalView::new(parser.screen());
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
    }
}
