use ratatui::{buffer::Buffer, layout::Rect};

use crate::renderer;

/// Render a vt100 screen into the given frame area.
pub fn render(buf: &mut Buffer, area: Rect, screen: &vt100::Screen) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    renderer::render_vt100_screen(buf, area, screen);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_empty_screen() {
        let parser = vt100::Parser::new(24, 80, 0);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, parser.screen());
        // Should not panic; buffer should be filled
    }

    #[test]
    fn test_render_screen_with_text() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Hello, gwt-tui!");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, parser.screen());
        assert_eq!(buf[(0, 0)].symbol(), "H");
        assert_eq!(buf[(1, 0)].symbol(), "e");
    }

    #[test]
    fn test_render_zero_area() {
        let parser = vt100::Parser::new(24, 80, 0);
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, parser.screen());
        // Should not panic
    }

    #[test]
    fn test_render_smaller_area_than_screen() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Wide content that exceeds the view area by a lot");
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, parser.screen());
        assert_eq!(buf[(0, 0)].symbol(), "W");
    }
}
