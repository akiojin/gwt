//! Message type — all actions in the Elm Architecture.

use crossterm::event::{KeyEvent, MouseEvent};
use gwt_core::logging::LogEvent;

use crate::input::voice::VoiceInputMessage;
use crate::model::ManagementTab;
use crate::screens::branches::BranchesMessage;
use crate::screens::cleanup_confirm::CleanupConfirmMessage;
use crate::screens::cleanup_progress::CleanupProgressMessage;
use crate::screens::confirm::ConfirmMessage;
use crate::screens::docker_progress::DockerProgressMessage;
use crate::screens::git_view::GitViewMessage;
use crate::screens::initialization::InitializationMessage;
use crate::screens::issues::IssuesMessage;
use crate::screens::logs::LogsMessage;
use crate::screens::port_select::PortSelectMessage;
use crate::screens::pr_dashboard::PrDashboardMessage;
use crate::screens::profiles::ProfilesMessage;
use crate::screens::service_select::ServiceSelectMessage;
use crate::screens::settings::SettingsMessage;
use crate::screens::versions::VersionsMessage;
use crate::screens::wizard::{SpecContext, WizardMessage};

/// Direction for moving the active session inside grid layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridSessionDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Every possible action in the TUI.
#[derive(Debug, Clone)]
pub enum Message {
    /// Quit the application.
    Quit,
    /// Toggle between Main (sessions) and Management layers.
    ToggleLayer,
    /// Move focus to the next logical pane.
    FocusNext,
    /// Move focus to the previous logical pane.
    FocusPrev,
    /// Switch to a specific management tab.
    SwitchManagementTab(ManagementTab),
    /// Activate the next session tab.
    NextSession,
    /// Activate the previous session tab.
    PrevSession,
    /// Switch to session by index (0-based).
    SwitchSession(usize),
    /// Move the active session selection within grid layout.
    MoveGridSession(GridSessionDirection),
    /// Toggle session layout between Tab and Grid.
    ToggleSessionLayout,
    /// Create a new shell session.
    NewShell,
    /// Close the active session.
    CloseSession,
    /// Raw key input forwarded to the active pane.
    KeyInput(KeyEvent),
    /// Bracketed paste payload from the outer terminal.
    PasteInput(String),
    /// Mouse input.
    MouseInput(MouseEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// PTY output arrived for a pane.
    PtyOutput(String, Vec<u8>),
    /// Periodic tick (100ms).
    Tick,
    /// Push a plain error message onto the queue.
    PushError(String),
    /// Push a structured error notification onto the queue.
    PushErrorNotification(LogEvent),
    /// Push a structured notification through the UI routing path.
    Notify(LogEvent),
    /// Show a structured notification in the status bar.
    ShowNotification(LogEvent),
    /// Dismiss the current status-bar notification.
    DismissNotification,
    /// Dismiss the top error.
    DismissError,
    /// Branches screen message.
    Branches(BranchesMessage),
    /// Profiles screen message.
    Profiles(ProfilesMessage),
    /// Issues screen message.
    Issues(IssuesMessage),
    /// Git view screen message.
    GitView(GitViewMessage),
    /// PR dashboard screen message.
    PrDashboard(PrDashboardMessage),
    /// Settings screen message.
    Settings(SettingsMessage),
    /// Logs screen message.
    Logs(LogsMessage),
    /// Versions screen message.
    Versions(VersionsMessage),
    /// Wizard overlay message.
    Wizard(WizardMessage),
    /// Docker progress overlay message.
    DockerProgress(DockerProgressMessage),
    /// Service selection overlay message.
    ServiceSelect(ServiceSelectMessage),
    /// Port selection overlay message.
    PortSelect(PortSelectMessage),
    /// Confirmation dialog message.
    Confirm(ConfirmMessage),
    /// Branch Cleanup confirm modal message.
    CleanupConfirm(CleanupConfirmMessage),
    /// Branch Cleanup progress modal message.
    CleanupProgress(CleanupProgressMessage),
    /// Voice input message.
    Voice(VoiceInputMessage),
    /// Initialization screen message.
    Initialization(InitializationMessage),
    /// Open active agent session conversion flow.
    OpenSessionConversion,
    /// Toggle the keybinding help overlay.
    ToggleHelp,
    /// Open the wizard overlay with SPEC context for prefilling.
    OpenWizardWithSpec(SpecContext),
    /// Close the wizard overlay.
    CloseWizard,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_variants_are_constructible() {
        let _ = Message::Quit;
        let _ = Message::ToggleLayer;
        let _ = Message::FocusNext;
        let _ = Message::FocusPrev;
        let _ = Message::SwitchManagementTab(ManagementTab::Branches);
        let _ = Message::NextSession;
        let _ = Message::PrevSession;
        let _ = Message::SwitchSession(0);
        let _ = Message::MoveGridSession(GridSessionDirection::Left);
        let _ = Message::ToggleSessionLayout;
        let _ = Message::NewShell;
        let _ = Message::CloseSession;
        let _ = Message::Tick;
        let _ = Message::PushError("err".into());
        let _ = Message::PushErrorNotification(LogEvent::new(
            gwt_core::logging::LogLevel::Error,
            "test",
            "err",
        ));
        let _ = Message::Notify(LogEvent::new(
            gwt_core::logging::LogLevel::Info,
            "test",
            "message",
        ));
        let _ = Message::ToggleHelp;
        let _ = Message::ShowNotification(LogEvent::new(
            gwt_core::logging::LogLevel::Warn,
            "test",
            "warning",
        ));
        let _ = Message::DismissNotification;
        let _ = Message::DismissError;
        let _ = Message::Resize(80, 24);
        let _ = Message::PtyOutput("id".into(), vec![0x41]);
        let _ = Message::PasteInput("git status".into());
        let _ = Message::Branches(BranchesMessage::MoveUp);
        let _ = Message::Profiles(ProfilesMessage::MoveUp);
        let _ = Message::Issues(IssuesMessage::MoveUp);
        let _ = Message::GitView(GitViewMessage::MoveUp);
        let _ = Message::PrDashboard(PrDashboardMessage::MoveUp);
        let _ = Message::Settings(SettingsMessage::MoveUp);
        let _ = Message::Logs(LogsMessage::MoveUp);
        let _ = Message::Versions(VersionsMessage::MoveUp);
        let _ = Message::Wizard(WizardMessage::MoveUp);
        let _ = Message::DockerProgress(DockerProgressMessage::Advance);
        let _ = Message::ServiceSelect(ServiceSelectMessage::MoveUp);
        let _ = Message::PortSelect(PortSelectMessage::MoveUp);
        let _ = Message::Confirm(ConfirmMessage::Toggle);
        let _ = Message::Voice(VoiceInputMessage::StartRecording);
        let _ = Message::Initialization(InitializationMessage::Exit);
        let _ = Message::OpenSessionConversion;
        let _ = Message::OpenWizardWithSpec(SpecContext::new("SPEC-1", "Title", ""));
        let _ = Message::CloseWizard;
    }
}
