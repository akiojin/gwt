//! Event handling for TUI

use crossterm::event::{KeyCode, KeyModifiers};

/// Key event representation
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    /// Create a new key event
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Check if this is a quit key (q or Ctrl+C twice)
    pub fn is_quit(&self) -> bool {
        matches!(self.code, KeyCode::Char('q'))
    }

    /// Check if this is a help key (? or h)
    pub fn is_help(&self) -> bool {
        matches!(self.code, KeyCode::Char('?') | KeyCode::Char('h'))
    }

    /// Check if this is Ctrl+C
    pub fn is_ctrl_c(&self) -> bool {
        matches!(self.code, KeyCode::Char('c'))
            && self.modifiers.contains(KeyModifiers::CONTROL)
    }

    /// Check if this is Enter
    pub fn is_enter(&self) -> bool {
        matches!(self.code, KeyCode::Enter)
    }

    /// Check if this is Escape
    pub fn is_escape(&self) -> bool {
        matches!(self.code, KeyCode::Esc)
    }

    /// Check if this is Up arrow
    pub fn is_up(&self) -> bool {
        matches!(self.code, KeyCode::Up)
    }

    /// Check if this is Down arrow
    pub fn is_down(&self) -> bool {
        matches!(self.code, KeyCode::Down)
    }

    /// Check if this is Page Up
    pub fn is_page_up(&self) -> bool {
        matches!(self.code, KeyCode::PageUp)
    }

    /// Check if this is Page Down
    pub fn is_page_down(&self) -> bool {
        matches!(self.code, KeyCode::PageDown)
    }

    /// Check if this is Home
    pub fn is_home(&self) -> bool {
        matches!(self.code, KeyCode::Home)
    }

    /// Check if this is End
    pub fn is_end(&self) -> bool {
        matches!(self.code, KeyCode::End)
    }
}
