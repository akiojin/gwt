//! VT100 to ratatui buffer renderer
//!
//! Converts VT100 cell buffer to ratatui Buffer for display.

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
};

use crate::terminal::emulator::TerminalEmulator;

/// Cursor position and visibility information
pub struct CursorInfo {
    pub row: u16,
    pub col: u16,
    pub visible: bool,
}

/// Convert a vt100::Color to a ratatui::style::Color
fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(n) => Color::Indexed(n),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Render the VT100 screen contents into a ratatui buffer
pub fn render_to_buffer(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    for y in 0..area.height {
        for x in 0..area.width {
            let vt_cell = match screen.cell(y, x) {
                Some(c) => c,
                None => continue,
            };

            if vt_cell.is_wide_continuation() {
                continue;
            }

            let pos = Position::new(area.x + x, area.y + y);
            let ratatui_cell = match buf.cell_mut(pos) {
                Some(c) => c,
                None => continue,
            };

            let contents = vt_cell.contents();
            if contents.is_empty() {
                ratatui_cell.set_symbol(" ");
            } else {
                ratatui_cell.set_symbol(contents);
            }

            let mut modifiers = Modifier::empty();
            if vt_cell.bold() {
                modifiers |= Modifier::BOLD;
            }
            if vt_cell.italic() {
                modifiers |= Modifier::ITALIC;
            }
            if vt_cell.underline() {
                modifiers |= Modifier::UNDERLINED;
            }
            if vt_cell.inverse() {
                modifiers |= Modifier::REVERSED;
            }

            let style = Style {
                fg: Some(convert_color(vt_cell.fgcolor())),
                bg: Some(convert_color(vt_cell.bgcolor())),
                add_modifier: modifiers,
                sub_modifier: Modifier::empty(),
                ..Style::default()
            };
            ratatui_cell.set_style(style);
        }
    }
}

/// Get cursor information from a terminal emulator
pub fn get_cursor_info(emulator: &TerminalEmulator) -> CursorInfo {
    let (row, col) = emulator.cursor_position();
    CursorInfo {
        row,
        col,
        visible: !emulator.hide_cursor(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::emulator::TerminalEmulator;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};

    /// Helper: create emulator, process bytes, render to buffer, return buffer
    fn render_emulator(rows: u16, cols: u16, input: &[u8]) -> (TerminalEmulator, Buffer) {
        let mut emu = TerminalEmulator::new(rows, cols);
        emu.process(input);
        let area = Rect::new(0, 0, cols, rows);
        let mut buf = Buffer::empty(area);
        render_to_buffer(emu.screen(), area, &mut buf);
        (emu, buf)
    }

    #[test]
    fn test_empty_screen() {
        let (_, buf) = render_emulator(24, 80, b"");
        // Empty screen cells should have empty or space content
        let cell = buf.cell(Position::new(0, 0)).expect("cell should exist");
        // vt100 empty cell contents is "" which we render as " "
        assert!(
            cell.symbol() == " " || cell.symbol() == "",
            "empty cell should be space or empty, got: {:?}",
            cell.symbol()
        );
    }

    #[test]
    fn test_text_rendering() {
        let (_, buf) = render_emulator(24, 80, b"hello");
        assert_eq!(buf.cell(Position::new(0, 0)).unwrap().symbol(), "h");
        assert_eq!(buf.cell(Position::new(1, 0)).unwrap().symbol(), "e");
        assert_eq!(buf.cell(Position::new(2, 0)).unwrap().symbol(), "l");
        assert_eq!(buf.cell(Position::new(3, 0)).unwrap().symbol(), "l");
        assert_eq!(buf.cell(Position::new(4, 0)).unwrap().symbol(), "o");
    }

    #[test]
    fn test_foreground_color() {
        // ESC[31m = red foreground (index 1)
        let (_, buf) = render_emulator(24, 80, b"\x1b[31mR");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1));
    }

    #[test]
    fn test_background_color() {
        // ESC[41m = red background (index 1)
        let (_, buf) = render_emulator(24, 80, b"\x1b[41mR");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.bg, Color::Indexed(1));
    }

    #[test]
    fn test_256_color() {
        // ESC[38;5;196m = foreground 256-color index 196
        let (_, buf) = render_emulator(24, 80, b"\x1b[38;5;196mx");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.fg, Color::Indexed(196));
    }

    #[test]
    fn test_truecolor() {
        // ESC[38;2;255;128;0m = foreground RGB(255,128,0)
        let (_, buf) = render_emulator(24, 80, b"\x1b[38;2;255;128;0mx");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.fg, Color::Rgb(255, 128, 0));
    }

    #[test]
    fn test_bold_attribute() {
        // ESC[1m = bold
        let (_, buf) = render_emulator(24, 80, b"\x1b[1mx");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "cell should have BOLD modifier"
        );
    }

    #[test]
    fn test_italic_attribute() {
        // ESC[3m = italic
        let (_, buf) = render_emulator(24, 80, b"\x1b[3mx");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert!(
            cell.modifier.contains(Modifier::ITALIC),
            "cell should have ITALIC modifier"
        );
    }

    #[test]
    fn test_underline_attribute() {
        // ESC[4m = underline
        let (_, buf) = render_emulator(24, 80, b"\x1b[4mx");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert!(
            cell.modifier.contains(Modifier::UNDERLINED),
            "cell should have UNDERLINED modifier"
        );
    }

    #[test]
    fn test_default_color_maps_to_reset() {
        assert_eq!(convert_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn test_convert_color_idx() {
        assert_eq!(convert_color(vt100::Color::Idx(42)), Color::Indexed(42));
    }

    #[test]
    fn test_convert_color_rgb() {
        assert_eq!(
            convert_color(vt100::Color::Rgb(10, 20, 30)),
            Color::Rgb(10, 20, 30)
        );
    }

    #[test]
    fn test_cursor_info() {
        let mut emu = TerminalEmulator::new(24, 80);
        emu.process(b"\x1b[5;10H"); // Move cursor to row 5, col 10 (1-based)
        let info = get_cursor_info(&emu);
        assert_eq!(info.row, 4); // 0-based
        assert_eq!(info.col, 9); // 0-based
        assert!(info.visible);
    }

    #[test]
    fn test_cursor_info_hidden() {
        let mut emu = TerminalEmulator::new(24, 80);
        emu.process(b"\x1b[?25l"); // Hide cursor
        let info = get_cursor_info(&emu);
        assert!(!info.visible);
    }

    #[test]
    fn test_inverse_attribute() {
        // ESC[7m = inverse/reverse
        let (_, buf) = render_emulator(24, 80, b"\x1b[7mx");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert!(
            cell.modifier.contains(Modifier::REVERSED),
            "cell should have REVERSED modifier"
        );
    }

    #[test]
    fn test_full_screen_render() {
        // Render a full 120x40 screen to check it completes without panic
        let (_, buf) = render_emulator(40, 120, b"full screen test");
        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.symbol(), "f");
    }
}
