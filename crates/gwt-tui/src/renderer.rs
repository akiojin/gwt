//! VT100 → ratatui renderer
//!
//! Converts a vt100::Screen into ratatui Buffer cells.
//! This module is kept as a placeholder; the full implementation
//! will be ported in Phase 2 from the gwt-cli reference.

use ratatui::prelude::*;

/// Render a vt100 screen into the given ratatui buffer area.
///
/// Each vt100 cell is converted to a ratatui Cell with matching
/// foreground/background colors and attributes.
pub fn render_vt100_screen(buf: &mut Buffer, area: Rect, screen: &vt100::Screen) {
    let rows = area.height as usize;
    let cols = area.width as usize;

    for row in 0..rows {
        for col in 0..cols {
            let vt_row = row as u16;
            let vt_col = col as u16;
            let cell = screen.cell(vt_row, vt_col);
            let buf_x = area.x + col as u16;
            let buf_y = area.y + row as u16;

            if let Some(cell) = cell {
                if let Some(buf_cell) = buf.cell_mut((buf_x, buf_y)) {
                    let ch = cell.contents();
                    if ch.is_empty() {
                        buf_cell.set_char(' ');
                    } else {
                        // set_symbol handles multi-char grapheme clusters
                        buf_cell.set_symbol(&ch);
                    }
                    buf_cell.set_style(vt100_to_ratatui_style(cell));
                }
            }
        }
    }
}

/// Convert vt100 cell colors/attributes to a ratatui Style.
fn vt100_to_ratatui_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default();
    style = style.fg(vt100_color_to_ratatui(cell.fgcolor()));
    style = style.bg(vt100_color_to_ratatui(cell.bgcolor()));

    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

/// Map vt100 color to ratatui Color.
fn vt100_color_to_ratatui(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
