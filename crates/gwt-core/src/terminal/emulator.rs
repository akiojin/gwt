//! VT100 terminal emulator wrapper
//!
//! Wraps the vt100 crate to provide terminal emulation.

use std::cell::Cell as StdCell;
use std::rc::Rc;

/// BEL detection callback for vt100 parser
#[derive(Clone)]
struct BellCallbacks {
    bell_pending: Rc<StdCell<bool>>,
}

impl BellCallbacks {
    fn new(bell_pending: Rc<StdCell<bool>>) -> Self {
        Self { bell_pending }
    }
}

impl vt100::Callbacks for BellCallbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell_pending.set(true);
    }
}

/// VT100 terminal emulator wrapping the vt100 crate
pub struct TerminalEmulator {
    parser: vt100::Parser<BellCallbacks>,
    bell_pending: Rc<StdCell<bool>>,
}

impl TerminalEmulator {
    /// Create a new terminal emulator with the given dimensions
    pub fn new(rows: u16, cols: u16) -> Self {
        let bell_pending = Rc::new(StdCell::new(false));
        let callbacks = BellCallbacks::new(Rc::clone(&bell_pending));
        let parser = vt100::Parser::new_with_callbacks(rows, cols, 0, callbacks);
        Self {
            parser,
            bell_pending,
        }
    }

    /// Process input bytes through the terminal emulator
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Get a cell at the given position
    pub fn cell(&self, row: u16, col: u16) -> Option<&vt100::Cell> {
        self.parser.screen().cell(row, col)
    }

    /// Get the current cursor position (row, col)
    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// Check if the cursor is hidden
    pub fn hide_cursor(&self) -> bool {
        self.parser.screen().hide_cursor()
    }

    /// Resize the terminal emulator
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// Get the current terminal size (rows, cols)
    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    /// Check if alternate screen is active
    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    /// Get the current mouse protocol mode
    pub fn mouse_protocol_mode(&self) -> vt100::MouseProtocolMode {
        self.parser.screen().mouse_protocol_mode()
    }

    /// Take the bell pending flag, returning its value and resetting it
    pub fn take_bell(&mut self) -> bool {
        let pending = self.bell_pending.get();
        self.bell_pending.set(false);
        pending
    }

    /// Get a reference to the internal screen
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_basic_text() {
        let mut emu = TerminalEmulator::new(24, 80);
        emu.process(b"hello");

        let cell_h = emu.cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell_h.contents(), "h");
        let cell_e = emu.cell(0, 1).expect("cell(0,1) should exist");
        assert_eq!(cell_e.contents(), "e");
        let cell_l1 = emu.cell(0, 2).expect("cell(0,2) should exist");
        assert_eq!(cell_l1.contents(), "l");
        let cell_l2 = emu.cell(0, 3).expect("cell(0,3) should exist");
        assert_eq!(cell_l2.contents(), "l");
        let cell_o = emu.cell(0, 4).expect("cell(0,4) should exist");
        assert_eq!(cell_o.contents(), "o");
    }

    #[test]
    fn test_ansi_foreground_color() {
        let mut emu = TerminalEmulator::new(24, 80);
        // ESC[31m = set foreground to red (index 1)
        emu.process(b"\x1b[31mred\x1b[0m");

        let cell = emu.cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell.contents(), "r");
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));
    }

    #[test]
    fn test_cursor_movement() {
        let mut emu = TerminalEmulator::new(24, 80);
        // ESC[5;10H = move cursor to row 5, col 10 (1-based) -> (4, 9) 0-based
        emu.process(b"\x1b[5;10H");

        assert_eq!(emu.cursor_position(), (4, 9));
    }

    #[test]
    fn test_screen_clear() {
        let mut emu = TerminalEmulator::new(24, 80);
        emu.process(b"hello");
        // ESC[2J = clear entire screen
        emu.process(b"\x1b[2J");

        let cell = emu.cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell.contents(), "");
    }

    #[test]
    fn test_resize() {
        let mut emu = TerminalEmulator::new(24, 80);
        assert_eq!(emu.size(), (24, 80));

        emu.resize(48, 120);
        assert_eq!(emu.size(), (48, 120));
    }

    #[test]
    fn test_bell_detection() {
        let mut emu = TerminalEmulator::new(24, 80);
        assert!(!emu.take_bell(), "bell should not be pending initially");

        // BEL character
        emu.process(b"\x07");
        assert!(emu.take_bell(), "bell should be pending after BEL");
        assert!(!emu.take_bell(), "bell should be cleared after take_bell()");
    }

    #[test]
    fn test_alternate_screen() {
        let mut emu = TerminalEmulator::new(24, 80);
        assert!(!emu.alternate_screen());

        // ESC[?1049h = enable alternate screen
        emu.process(b"\x1b[?1049h");
        assert!(emu.alternate_screen());

        // ESC[?1049l = disable alternate screen
        emu.process(b"\x1b[?1049l");
        assert!(!emu.alternate_screen());
    }

    #[test]
    fn test_256_color() {
        let mut emu = TerminalEmulator::new(24, 80);
        // ESC[38;5;196m = set foreground to 256-color index 196
        emu.process(b"\x1b[38;5;196mx\x1b[0m");

        let cell = emu.cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell.contents(), "x");
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(196));
    }

    #[test]
    fn test_truecolor() {
        let mut emu = TerminalEmulator::new(24, 80);
        // ESC[38;2;255;128;0m = set foreground to RGB(255,128,0)
        emu.process(b"\x1b[38;2;255;128;0mx\x1b[0m");

        let cell = emu.cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell.contents(), "x");
        assert_eq!(cell.fgcolor(), vt100::Color::Rgb(255, 128, 0));
    }

    #[test]
    fn test_text_attributes() {
        let mut emu = TerminalEmulator::new(24, 80);
        // ESC[1m = bold, ESC[3m = italic, ESC[4m = underline
        emu.process(b"\x1b[1;3;4mx\x1b[0m");

        let cell = emu.cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell.contents(), "x");
        assert!(cell.bold(), "cell should be bold");
        assert!(cell.italic(), "cell should be italic");
        assert!(cell.underline(), "cell should be underlined");
    }

    #[test]
    fn test_hide_cursor() {
        let mut emu = TerminalEmulator::new(24, 80);
        assert!(!emu.hide_cursor());

        // ESC[?25l = hide cursor
        emu.process(b"\x1b[?25l");
        assert!(emu.hide_cursor());

        // ESC[?25h = show cursor
        emu.process(b"\x1b[?25h");
        assert!(!emu.hide_cursor());
    }

    #[test]
    fn test_mouse_protocol_mode() {
        let mut emu = TerminalEmulator::new(24, 80);
        assert_eq!(emu.mouse_protocol_mode(), vt100::MouseProtocolMode::None);

        // ESC[?1000h = enable mouse press/release reporting
        emu.process(b"\x1b[?1000h");
        assert_eq!(
            emu.mouse_protocol_mode(),
            vt100::MouseProtocolMode::PressRelease
        );
    }

    #[test]
    fn test_screen_access() {
        let emu = TerminalEmulator::new(24, 80);
        let screen = emu.screen();
        assert_eq!(screen.size(), (24, 80));
    }

    #[test]
    fn test_new_default_size() {
        let emu = TerminalEmulator::new(24, 80);
        assert_eq!(emu.size(), (24, 80));
        assert_eq!(emu.cursor_position(), (0, 0));
    }

    #[test]
    fn test_bell_multiple_times() {
        let mut emu = TerminalEmulator::new(24, 80);
        emu.process(b"\x07\x07\x07");
        assert!(
            emu.take_bell(),
            "bell should be pending after multiple BELs"
        );
        assert!(!emu.take_bell(), "bell should be cleared after take");
    }
}
