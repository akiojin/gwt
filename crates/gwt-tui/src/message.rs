//! Message type — all actions in the Elm Architecture.

use crossterm::event::{KeyEvent, MouseEvent};

use crate::model::ManagementTab;
use crate::input::voice::VoiceInputMessage;
use crate::screens::branches::BranchesMessage;
use crate::screens::confirm::ConfirmMessage;
use crate::screens::docker_progress::DockerProgressMessage;
use crate::screens::git_view::GitViewMessage;
use crate::screens::issues::IssuesMessage;
use crate::screens::logs::LogsMessage;
use crate::screens::port_select::PortSelectMessage;
use crate::screens::pr_dashboard::PrDashboardMessage;
use crate::screens::profiles::ProfilesMessage;
use crate::screens::service_select::ServiceSelectMessage;
use crate::screens::settings::SettingsMessage;
use crate::screens::specs::SpecsMessage;
use crate::screens::versions::VersionsMessage;
use crate::screens::wizard::WizardMessage;

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
    /// Specs screen message.
    Specs(SpecsMessage),
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
    /// Voice input message.
    Voice(VoiceInputMessage),
    /// Paste file paths from clipboard.
    PasteFiles,
    /// Open the wizard overlay.
    OpenWizard,
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
        let _ = Message::Branches(BranchesMessage::MoveUp);
        let _ = Message::Profiles(ProfilesMessage::MoveUp);
        let _ = Message::Issues(IssuesMessage::MoveUp);
        let _ = Message::GitView(GitViewMessage::MoveUp);
        let _ = Message::PrDashboard(PrDashboardMessage::MoveUp);
        let _ = Message::Specs(SpecsMessage::MoveUp);
        let _ = Message::Settings(SettingsMessage::MoveUp);
        let _ = Message::Logs(LogsMessage::MoveUp);
        let _ = Message::Versions(VersionsMessage::MoveUp);
        let _ = Message::Wizard(WizardMessage::MoveUp);
        let _ = Message::DockerProgress(DockerProgressMessage::Advance);
        let _ = Message::ServiceSelect(ServiceSelectMessage::MoveUp);
        let _ = Message::PortSelect(PortSelectMessage::MoveUp);
        let _ = Message::Confirm(ConfirmMessage::Toggle);
        let _ = Message::Voice(VoiceInputMessage::StartRecording);
        let _ = Message::PasteFiles;
        let _ = Message::OpenWizard;
        let _ = Message::CloseWizard;
    }
}
