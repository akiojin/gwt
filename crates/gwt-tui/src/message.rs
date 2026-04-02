//! Message enum for Elm Architecture update loop

use crossterm::event::{KeyEvent, MouseEvent};

use crate::model::{ErrorEntry, ManagementTab};
use crate::screens::versions::VersionsMessage;
use crate::screens::{BranchesMessage, IssuesMessage, LogsMessage, SettingsMessage};

/// All messages that can flow through the Elm Architecture update loop.
#[derive(Debug)]
pub enum Message {
    // -- Navigation -----------------------------------------------------------
    /// Quit the application
    Quit,
    /// Toggle between Main and Management layers (Ctrl+G, Ctrl+G)
    ToggleLayer,
    /// Switch to a specific management tab
    SwitchManagementTab(ManagementTab),

    // -- Session tab management -----------------------------------------------
    /// Next session tab (Ctrl+G, ])
    NextSession,
    /// Previous session tab (Ctrl+G, [)
    PrevSession,
    /// Switch to session by 1-based index (Ctrl+G, 1-9)
    SwitchSession(usize),
    /// Toggle between equal-grid and maximized session workspace
    ToggleSessionLayout,
    /// Close current session (Ctrl+G, &)
    CloseSession,
    /// Open a new shell tab (Ctrl+G, c)
    NewShell,
    /// Legacy shortcut: return the active terminal viewport to the live tail
    TogglePtyCopyMode,

    // -- Input events ---------------------------------------------------------
    /// Raw key input (forwarded to active pane or screen handler)
    KeyInput(KeyEvent),
    /// Pasted text input
    Paste(String),
    /// Mouse input
    MouseInput(MouseEvent),
    /// Terminal resize
    Resize(u16, u16),

    // -- PTY ------------------------------------------------------------------
    /// Output from a PTY pane
    PtyOutput {
        pane_id: String,
        data: Vec<u8>,
    },

    // -- Tick -----------------------------------------------------------------
    /// Periodic tick (~250ms) for background polling
    Tick,

    // -- Errors ---------------------------------------------------------------
    /// Push a new error onto the queue
    PushError(ErrorEntry),
    /// Dismiss the front-most error
    DismissError,

    // -- Wizard ----------------------------------------------------------------
    /// Key input forwarded to wizard overlay
    WizardKey(KeyEvent),

    // -- Overlays / dialogs ---------------------------------------------------
    /// Open the clone wizard
    OpenCloneWizard,
    /// Close the clone wizard
    CloseCloneWizard,
    /// Open the speckit wizard
    OpenSpecKitWizard,
    /// Close the speckit wizard
    CloseSpecKitWizard,
    /// Confirm dialog accepted
    ConfirmAccepted,
    /// Confirm dialog cancelled
    ConfirmCancelled,
    /// Progress advance
    ProgressAdvance,
    /// Progress error
    ProgressError(String),

    // -- Screen-specific messages (delegated) ---------------------------------
    BranchesMsg(BranchesMessage),
    IssuesMsg(IssuesMessage),
    SettingsMsg(SettingsMessage),
    LogsMsg(LogsMessage),
    VersionsMsg(VersionsMessage),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ErrorSeverity;

    #[test]
    fn message_variants_are_constructible() {
        // Ensure key variants compile and can be pattern-matched
        let msgs: Vec<Message> = vec![
            Message::Quit,
            Message::ToggleLayer,
            Message::SwitchManagementTab(ManagementTab::Branches),
            Message::NextSession,
            Message::PrevSession,
            Message::SwitchSession(0),
            Message::ToggleSessionLayout,
            Message::CloseSession,
            Message::NewShell,
            Message::TogglePtyCopyMode,
            Message::WizardKey(KeyEvent {
                code: crossterm::event::KeyCode::Enter,
                modifiers: crossterm::event::KeyModifiers::NONE,
                kind: crossterm::event::KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            }),
            Message::Paste("hello\nworld".to_string()),
            Message::Resize(80, 24),
            Message::PtyOutput {
                pane_id: "p1".into(),
                data: vec![0x41],
            },
            Message::Tick,
            Message::PushError(ErrorEntry {
                message: "err".into(),
                severity: ErrorSeverity::Minor,
            }),
            Message::DismissError,
            Message::OpenCloneWizard,
            Message::CloseCloneWizard,
            Message::OpenSpecKitWizard,
            Message::CloseSpecKitWizard,
            Message::ConfirmAccepted,
            Message::ConfirmCancelled,
            Message::ProgressAdvance,
            Message::ProgressError("err".into()),
            Message::BranchesMsg(BranchesMessage::Refresh),
            Message::IssuesMsg(IssuesMessage::Refresh),
            Message::SettingsMsg(SettingsMessage::Refresh),
            Message::LogsMsg(LogsMessage::Refresh),
            Message::VersionsMsg(VersionsMessage::SelectNext),
        ];
        assert!(msgs.len() > 10);
    }

    #[test]
    fn message_is_debug() {
        let msg = Message::Quit;
        let s = format!("{msg:?}");
        assert!(s.contains("Quit"));
    }
}
