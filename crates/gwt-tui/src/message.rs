//! Message type — all actions in the Elm Architecture.

use crossterm::event::{KeyEvent, MouseEvent};

use crate::model::ManagementTab;

/// Every possible action in the TUI.
#[derive(Debug, Clone)]
pub enum Message {
    /// Quit the application.
    Quit,
    /// Toggle between Main (sessions) and Management layers.
    ToggleLayer,
    /// Switch to a specific management tab.
    SwitchManagementTab(ManagementTab),
    /// Activate the next session tab.
    NextSession,
    /// Activate the previous session tab.
    PrevSession,
    /// Switch to session by index (0-based).
    SwitchSession(usize),
    /// Toggle session layout between Tab and Grid.
    ToggleSessionLayout,
    /// Create a new shell session.
    NewShell,
    /// Close the active session.
    CloseSession,
    /// Raw key input forwarded to the active pane.
    KeyInput(KeyEvent),
    /// Mouse input.
    MouseInput(MouseEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// PTY output arrived for a pane.
    PtyOutput(String, Vec<u8>),
    /// Periodic tick (100ms).
    Tick,
    /// Push an error message onto the queue.
    PushError(String),
    /// Dismiss the top error.
    DismissError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_variants_are_constructible() {
        let _ = Message::Quit;
        let _ = Message::ToggleLayer;
        let _ = Message::SwitchManagementTab(ManagementTab::Branches);
        let _ = Message::NextSession;
        let _ = Message::PrevSession;
        let _ = Message::SwitchSession(0);
        let _ = Message::ToggleSessionLayout;
        let _ = Message::NewShell;
        let _ = Message::CloseSession;
        let _ = Message::Tick;
        let _ = Message::PushError("err".into());
        let _ = Message::DismissError;
        let _ = Message::Resize(80, 24);
        let _ = Message::PtyOutput("id".into(), vec![0x41]);
    }
}
