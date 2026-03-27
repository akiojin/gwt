use ratatui::{
    buffer::Buffer, layout::Rect, style::Color as RatColor, style::Modifier, style::Style,
};

/// Convert a vt100 color to a ratatui color.
pub fn convert_color(color: vt100::Color) -> RatColor {
    match color {
        vt100::Color::Default => RatColor::Reset,
        vt100::Color::Idx(i) => RatColor::Indexed(i),
        vt100::Color::Rgb(r, g, b) => RatColor::Rgb(r, g, b),
    }
}

/// Convert vt100 cell attributes to ratatui modifier.
pub fn convert_attrs(cell: &vt100::Cell) -> Modifier {
    let mut modifier = Modifier::empty();
    if cell.bold() {
        modifier |= Modifier::BOLD;
    }
    if cell.italic() {
        modifier |= Modifier::ITALIC;
    }
    if cell.underline() {
        modifier |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        modifier |= Modifier::REVERSED;
    }
    modifier
}

/// Render a vt100 screen to a ratatui buffer.
pub fn render_screen(screen: &vt100::Screen, area: Rect) -> Buffer {
    let mut buffer = Buffer::empty(area);
    let rows = area.height.min(screen.size().0);
    let cols = area.width.min(screen.size().1);

    for row in 0..rows {
        for col in 0..cols {
            let cell = screen.cell(row, col);
            if let Some(cell) = cell {
                let fg = convert_color(cell.fgcolor());
                let bg = convert_color(cell.bgcolor());
                let modifier = convert_attrs(cell);
                let style = Style::default().fg(fg).bg(bg).add_modifier(modifier);
                let ch = cell.contents();
                if !ch.is_empty() {
                    let buf_cell = &mut buffer[(area.x + col, area.y + row)];
                    buf_cell.set_symbol(&ch);
                    buf_cell.set_style(style);
                }
            }
        }
    }

    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_default_color() {
        assert_eq!(convert_color(vt100::Color::Default), RatColor::Reset);
    }

    #[test]
    fn test_convert_indexed_color() {
        assert_eq!(
            convert_color(vt100::Color::Idx(196)),
            RatColor::Indexed(196)
        );
    }

    #[test]
    fn test_convert_rgb_color() {
        assert_eq!(
            convert_color(vt100::Color::Rgb(255, 128, 0)),
            RatColor::Rgb(255, 128, 0)
        );
    }

    #[test]
    fn test_render_empty_screen() {
        let parser = vt100::Parser::new(24, 80, 0);
        let screen = parser.screen();
        let area = Rect::new(0, 0, 80, 24);
        let buffer = render_screen(screen, area);
        assert_eq!(buffer.area, area);
    }

    #[test]
    fn test_render_screen_with_text() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"Hello, world!");
        let screen = parser.screen();
        let area = Rect::new(0, 0, 80, 24);
        let buffer = render_screen(screen, area);
        let cell = &buffer[(0, 0)];
        assert_eq!(cell.symbol(), "H");
    }

    #[test]
    fn test_render_screen_with_ansi_colors() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        // Red foreground text
        parser.process(b"\x1b[31mRed\x1b[0m");
        let screen = parser.screen();
        let area = Rect::new(0, 0, 80, 24);
        let buffer = render_screen(screen, area);
        let cell = &buffer[(0, 0)];
        assert_eq!(cell.symbol(), "R");
        // vt100 maps color 31 (red) to Idx(1)
        assert_eq!(cell.style().fg, Some(RatColor::Indexed(1)));
    }

    #[test]
    fn test_render_screen_with_bold() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[1mBold\x1b[0m");
        let screen = parser.screen();
        let area = Rect::new(0, 0, 80, 24);
        let buffer = render_screen(screen, area);
        let cell = &buffer[(0, 0)];
        assert!(cell.style().add_modifier.contains(Modifier::BOLD));
    }
}
