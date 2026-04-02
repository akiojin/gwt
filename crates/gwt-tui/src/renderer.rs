//! Renderer — converts vt100 screen cells to ratatui Buffer.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

/// Map a vt100 color to a ratatui color.
pub fn map_vt_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Map vt100 cell attributes to ratatui Modifier.
pub fn map_vt_attrs(bold: bool, italic: bool, underline: bool, inverse: bool) -> Modifier {
    let mut mods = Modifier::empty();
    if bold {
        mods |= Modifier::BOLD;
    }
    if italic {
        mods |= Modifier::ITALIC;
    }
    if underline {
        mods |= Modifier::UNDERLINED;
    }
    if inverse {
        mods |= Modifier::REVERSED;
    }
    mods
}

/// Render a vt100 screen into a ratatui Buffer at the given area.
pub fn render_vt_screen(screen: &vt100::Screen, buf: &mut Buffer, area: Rect) {
    let rows = area.height.min(screen.size().0);
    let cols = area.width.min(screen.size().1);

    for row in 0..rows {
        for col in 0..cols {
            let cell = screen.cell(row, col);
            if let Some(cell) = cell {
                let x = area.x + col;
                let y = area.y + row;

                if x < buf.area().right() && y < buf.area().bottom() {
                    let buf_cell = &mut buf[(x, y)];
                    buf_cell.set_char(cell.contents().chars().next().unwrap_or(' '));
                    buf_cell.set_style(Style {
                        fg: Some(map_vt_color(cell.fgcolor())),
                        bg: Some(map_vt_color(cell.bgcolor())),
                        underline_color: None,
                        add_modifier: map_vt_attrs(
                            cell.bold(),
                            cell.italic(),
                            cell.underline(),
                            cell.inverse(),
                        ),
                        sub_modifier: Modifier::empty(),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_vt_color_default() {
        assert_eq!(map_vt_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn map_vt_color_indexed() {
        assert_eq!(map_vt_color(vt100::Color::Idx(42)), Color::Indexed(42));
    }

    #[test]
    fn map_vt_color_rgb() {
        assert_eq!(
            map_vt_color(vt100::Color::Rgb(10, 20, 30)),
            Color::Rgb(10, 20, 30)
        );
    }

    #[test]
    fn map_vt_attrs_none() {
        assert_eq!(map_vt_attrs(false, false, false, false), Modifier::empty());
    }

    #[test]
    fn map_vt_attrs_all() {
        let m = map_vt_attrs(true, true, true, true);
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::ITALIC));
        assert!(m.contains(Modifier::UNDERLINED));
        assert!(m.contains(Modifier::REVERSED));
    }

    #[test]
    fn render_vt_screen_basic() {
        let parser = vt100::Parser::new(2, 3, 0);
        let screen = parser.screen();
        let area = Rect::new(0, 0, 3, 2);
        let mut buf = Buffer::empty(area);
        render_vt_screen(screen, &mut buf, area);
        // Should not panic — cells are blank
    }
}
