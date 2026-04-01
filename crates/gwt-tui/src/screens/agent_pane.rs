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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::prelude::*;

    #[test]
    fn render_none_shows_starting_placeholder() {
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, None);
        assert!(result.is_none());
        // "Starting..." should appear somewhere in the buffer
        let text: String = (0..area.width)
            .map(|x| buf.cell((x, area.y)).map_or(' ', |c| {
                c.symbol().chars().next().unwrap_or(' ')
            }))
            .collect();
        // The paragraph is center-aligned, so it may have padding
        assert!(
            text.contains("Starting..."),
            "Expected 'Starting...' in buffer, got: {:?}",
            text.trim()
        );
    }

    #[test]
    fn render_with_parser_no_cursor() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Hello, world!");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, Some(&parser));
        // Cursor is visible by default in vt100, so we get Some
        assert!(result.is_some());
    }

    #[test]
    fn render_with_parser_hidden_cursor() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        // ESC[?25l hides cursor
        parser.process(b"\x1b[?25lHidden cursor");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, Some(&parser));
        assert!(result.is_none());
    }

    #[test]
    fn render_with_parser_cursor_position() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        // Move cursor to row 2, col 5 (1-based in ANSI)
        parser.process(b"\x1b[3;6HX");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let result = render(&mut buf, area, Some(&parser));
        if let Some((x, y)) = result {
            // Cursor is after the 'X' we wrote at (row=2, col=6) in 0-based
            assert!(x < area.right());
            assert!(y < area.bottom());
        }
    }

    #[test]
    fn render_with_parser_text_appears_in_buffer() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"TestOutput");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, Some(&parser));
        let row: String = (0..10)
            .map(|x| buf.cell((x, 0)).map_or(' ', |c| {
                c.symbol().chars().next().unwrap_or(' ')
            }))
            .collect();
        assert_eq!(row, "TestOutput");
    }
}
