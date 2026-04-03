//! Model — central application state for the Elm Architecture.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::input::voice::VoiceInputState;
use crate::screens::branches::BranchesState;
use crate::screens::confirm::ConfirmState;
use crate::screens::docker_progress::DockerProgressState;
use crate::screens::git_view::GitViewState;
use crate::screens::initialization::InitializationState;
use crate::screens::issues::IssuesState;
use crate::screens::logs::LogsState;
use crate::screens::port_select::PortSelectState;
use crate::screens::pr_dashboard::PrDashboardState;
use crate::screens::profiles::ProfilesState;
use crate::screens::service_select::ServiceSelectState;
use crate::screens::settings::SettingsState;
use crate::screens::versions::VersionsState;
use crate::screens::wizard::WizardState;
use gwt_notification::{Notification, NotificationBus, NotificationReceiver, StructuredLog};

/// Which UI layer is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayer {
    /// Initialization screen (no repo detected — clone wizard or bare migration).
    Initialization,
    /// Session panes (shell / agent terminals).
    Main,
    /// Management panel (branches, specs, issues, etc.).
    Management,
}

/// Which pane currently owns keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusPane {
    /// Management tab header (Left/Right switches tabs).
    TabHeader,
    /// Tab content area (↑↓ navigates list).
    #[default]
    TabContent,
    /// Branch detail panel (←→ sections, ↑↓ actions).
    BranchDetail,
    /// Terminal PTY (all keys forwarded).
    Terminal,
}

impl FocusPane {
    const ALL: [FocusPane; 4] = [
        FocusPane::TabHeader,
        FocusPane::TabContent,
        FocusPane::BranchDetail,
        FocusPane::Terminal,
    ];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&p| p == self).unwrap_or(0);
        Self::ALL[if idx == 0 { Self::ALL.len() - 1 } else { idx - 1 }]
    }
}

/// Session layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLayout {
    /// One session visible at a time.
    Tab,
    /// All sessions in an equal grid.
    Grid,
}

/// Management panel tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementTab {
    Branches,
    Issues,
    PrDashboard,
    Profiles,
    GitView,
    Versions,
    Settings,
    Logs,
}

impl ManagementTab {
    /// All tabs in display order.
    pub const ALL: [ManagementTab; 8] = [
        ManagementTab::Branches,
        ManagementTab::Issues,
        ManagementTab::PrDashboard,
        ManagementTab::Profiles,
        ManagementTab::GitView,
        ManagementTab::Versions,
        ManagementTab::Settings,
        ManagementTab::Logs,
    ];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Branches => "Branches",
            Self::Issues => "Issues",
            Self::PrDashboard => "PRs",
            Self::Profiles => "Profiles",
            Self::GitView => "Git View",
            Self::Versions => "Versions",
            Self::Settings => "Settings",
            Self::Logs => "Logs",
        }
    }
}

/// Type of a session tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTabType {
    Shell,
    Agent { agent_id: String, color: AgentColor },
}

/// Agent color for TUI display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentColor {
    Green,
    Blue,
    Cyan,
    Yellow,
    Magenta,
    Gray,
}

/// A single session tab (shell or agent).
#[derive(Debug, Clone)]
pub struct SessionTab {
    pub id: String,
    pub name: String,
    pub tab_type: SessionTabType,
    pub vt: VtState,
}

/// Buffered PTY input waiting to be written to the active session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPtyInput {
    pub session_id: String,
    pub bytes: Vec<u8>,
}

/// Pending session conversion selected from the overlay flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSessionConversion {
    pub session_index: usize,
    pub target_agent_id: String,
    pub target_display_name: String,
}

/// Minimal vt100 screen state wrapper.
#[derive(Debug, Clone)]
pub struct VtState {
    rows: u16,
    cols: u16,
}

impl VtState {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }
}

/// Central application state.
#[derive(Debug)]
pub struct Model {
    /// Active status-bar notification (Info/Warn surface).
    pub(crate) current_notification: Option<Notification>,
    /// Remaining lifetime for auto-dismissing status notifications.
    pub(crate) current_notification_ttl: Option<Duration>,
    /// Structured notification log.
    pub(crate) notification_log: StructuredLog,
    /// Sender side of the notification bus.
    pub(crate) _notification_bus: NotificationBus,
    /// Receiver side of the notification bus.
    pub(crate) notification_receiver: NotificationReceiver,
    /// Which layer has focus.
    pub active_layer: ActiveLayer,
    /// Which pane has keyboard focus.
    pub active_focus: FocusPane,
    /// All open session tabs.
    pub(crate) sessions: Vec<SessionTab>,
    /// Index of the active session.
    pub(crate) active_session: usize,
    /// Session layout mode.
    pub session_layout: SessionLayout,
    /// Active management tab.
    pub management_tab: ManagementTab,
    /// Error queue (shown as overlays).
    pub(crate) error_queue: VecDeque<String>,
    /// Whether the app should quit.
    pub quit: bool,
    /// Repository path.
    pub(crate) repo_path: PathBuf,
    /// Terminal size.
    pub(crate) terminal_size: (u16, u16),
    /// Branches screen state.
    pub(crate) branches: BranchesState,
    /// Profiles screen state.
    pub(crate) profiles: ProfilesState,
    /// Issues screen state.
    pub(crate) issues: IssuesState,
    /// Git view screen state.
    pub(crate) git_view: GitViewState,
    /// PR dashboard screen state.
    pub(crate) pr_dashboard: PrDashboardState,
    /// Settings screen state.
    pub(crate) settings: SettingsState,
    /// Logs screen state.
    pub(crate) logs: LogsState,
    /// Versions screen state.
    pub(crate) versions: VersionsState,
    /// Wizard overlay state (None when not active).
    pub(crate) wizard: Option<WizardState>,
    /// Docker progress overlay state.
    pub(crate) docker_progress: Option<DockerProgressState>,
    /// Service selection overlay state.
    pub(crate) service_select: Option<ServiceSelectState>,
    /// Port conflict resolution overlay state.
    pub(crate) port_select: Option<PortSelectState>,
    /// Confirmation dialog state.
    pub(crate) confirm: ConfirmState,
    /// Pending session conversion awaiting confirmation.
    pub(crate) pending_session_conversion: Option<PendingSessionConversion>,
    /// Voice input state.
    pub(crate) voice: VoiceInputState,
    /// Buffered PTY input generated from forwarded key events.
    pub(crate) pending_pty_inputs: VecDeque<PendingPtyInput>,
    /// Initialization screen state (present when ActiveLayer::Initialization).
    pub(crate) initialization: Option<InitializationState>,
}

impl Model {
    /// Create a new Model with sensible defaults.
    pub fn new(repo_path: PathBuf) -> Self {
        let default_session = SessionTab {
            id: "shell-0".to_string(),
            name: "Shell".to_string(),
            tab_type: SessionTabType::Shell,
            vt: VtState::new(24, 80),
        };
        let (notification_bus, notification_receiver) = NotificationBus::new();

        Self {
            current_notification: None,
            current_notification_ttl: None,
            notification_log: StructuredLog::default(),
            _notification_bus: notification_bus,
            notification_receiver,
            active_layer: ActiveLayer::Management,
            active_focus: FocusPane::TabContent,
            sessions: vec![default_session],
            active_session: 0,
            session_layout: SessionLayout::Tab,
            management_tab: ManagementTab::Branches,
            error_queue: VecDeque::new(),
            quit: false,
            repo_path,
            terminal_size: (80, 24),
            branches: BranchesState::default(),
            profiles: ProfilesState::default(),
            issues: IssuesState::default(),
            git_view: GitViewState::default(),
            pr_dashboard: PrDashboardState::default(),
            settings: SettingsState::default(),
            logs: LogsState::default(),
            versions: VersionsState::default(),
            wizard: None,
            docker_progress: None,
            service_select: None,
            port_select: None,
            confirm: ConfirmState::default(),
            pending_session_conversion: None,
            voice: VoiceInputState::default(),
            pending_pty_inputs: VecDeque::new(),
            initialization: None,
        }
    }

    /// Create a new Model in Initialization layer (no repo detected).
    pub fn new_initialization(repo_path: PathBuf, bare_migration: bool) -> Self {
        let mut model = Self::new(repo_path);
        model.active_layer = ActiveLayer::Initialization;
        model.initialization = Some(InitializationState::new(bare_migration));
        model
    }

    /// Reset all state for a new repository path (after successful clone).
    ///
    /// Transitions to Management layer, discarding all previous state.
    pub fn reset(&mut self, repo_path: PathBuf) {
        let terminal_size = self.terminal_size;
        *self = Self::new(repo_path);
        self.terminal_size = terminal_size;
    }

    /// Number of sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get the active session, if any.
    pub fn active_session_tab(&self) -> Option<&SessionTab> {
        self.sessions.get(self.active_session)
    }

    /// Buffered PTY input awaiting delivery to sessions.
    pub fn pending_pty_inputs(&self) -> &VecDeque<PendingPtyInput> {
        &self.pending_pty_inputs
    }

    /// Cloneable handle for sending notifications into the TUI.
    #[allow(dead_code)]
    pub(crate) fn notification_bus_handle(&self) -> NotificationBus {
        self._notification_bus.clone()
    }

    /// Drain queued notifications from the in-process bus.
    pub(crate) fn drain_notifications(&mut self) -> Vec<Notification> {
        self.notification_receiver.drain()
    }

    /// Get a mutable reference to the initialization state.
    pub fn initialization_mut(&mut self) -> Option<&mut InitializationState> {
        self.initialization.as_mut()
    }

    /// Get a reference to the initialization state.
    pub fn initialization(&self) -> Option<&InitializationState> {
        self.initialization.as_ref()
    }

    /// Whether a wizard overlay is active.
    pub fn has_wizard(&self) -> bool {
        self.wizard.is_some()
    }

    /// Whether the branches search is active.
    pub fn is_branches_search_active(&self) -> bool {
        self.branches.search_active
    }

    /// Current branches search query.
    pub fn branches_search_query(&self) -> &str {
        &self.branches.search_query
    }

    /// Active detail section index for the branches screen.
    pub fn branches_detail_section(&self) -> usize {
        self.branches.detail_section
    }

    /// Whether the branch detail launch-agent action is pending.
    pub fn branches_pending_launch_agent(&self) -> bool {
        self.branches.pending_launch_agent
    }

    /// Filtered branch names in display order.
    pub fn filtered_branch_names(&self) -> Vec<String> {
        self.branches
            .filtered_branches()
            .into_iter()
            .map(|branch| branch.name.clone())
            .collect()
    }

    /// Save session state to a TOML file. Best-effort: errors are logged, not fatal.
    pub fn save_session_state(&self, path: &Path) -> Result<(), String> {
        let display_mode = match self.session_layout {
            SessionLayout::Tab => "tab",
            SessionLayout::Grid => "grid",
        };
        let management_visible = self.active_layer == ActiveLayer::Management;
        let content = format!(
            "display_mode = \"{}\"\nmanagement_visible = {}\nactive_management_tab = \"{}\"\nsession_count = {}\n",
            display_mode, management_visible, self.management_tab.label(), self.sessions.len(),
        );
        std::fs::write(path, content).map_err(|e| e.to_string())
    }

    /// Load session state from a TOML file. Returns None on any error.
    pub fn load_session_state(path: &Path) -> Option<SessionState> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut state = SessionState::default();
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("display_mode = ") {
                state.display_mode = val.trim_matches('"').to_string();
            } else if let Some(val) = line.strip_prefix("management_visible = ") {
                state.management_visible = val == "true";
            } else if let Some(val) = line.strip_prefix("active_management_tab = ") {
                state.active_management_tab = val.trim_matches('"').to_string();
            } else if let Some(val) = line.strip_prefix("session_count = ") {
                state.session_count = val.parse().unwrap_or(0);
            }
        }
        Some(state)
    }
}

/// Persisted session layout state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub display_mode: String,
    pub management_visible: bool,
    pub active_management_tab: String,
    pub session_count: usize,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            display_mode: "tab".to_string(),
            management_visible: false,
            active_management_tab: "Branches".to_string(),
            session_count: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn model_new_defaults() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.session_count(), 1);
        assert_eq!(model.active_session, 0);
        assert_eq!(model.session_layout, SessionLayout::Tab);
        assert_eq!(model.management_tab, ManagementTab::Branches);
        assert!(model.error_queue.is_empty());
        assert!(!model.quit);
        assert!(model.drain_notifications().is_empty());
        assert!(model.notification_bus_handle().send(Notification::new(
            gwt_notification::Severity::Info,
            "test",
            "queued",
        )));
    }

    #[test]
    fn active_session_tab_returns_first() {
        let model = Model::new(PathBuf::from("/tmp/repo"));
        let tab = model.active_session_tab().unwrap();
        assert_eq!(tab.name, "Shell");
        assert_eq!(tab.tab_type, SessionTabType::Shell);
    }

    #[test]
    fn management_tab_labels() {
        assert_eq!(ManagementTab::Branches.label(), "Branches");
        assert_eq!(ManagementTab::Settings.label(), "Settings");
        assert_eq!(ManagementTab::Logs.label(), "Logs");
    }

    #[test]
    fn management_tab_all_has_eight_entries() {
        assert_eq!(ManagementTab::ALL.len(), 8);
    }

    #[test]
    fn vt_state_dimensions() {
        let vt = VtState::new(40, 120);
        assert_eq!(vt.rows(), 40);
        assert_eq!(vt.cols(), 120);
    }

    // ---- SessionState tests ----

    #[test]
    fn session_state_default() {
        let state = SessionState::default();
        assert_eq!(state.display_mode, "tab");
        assert!(!state.management_visible);
        assert_eq!(state.active_management_tab, "Branches");
        assert_eq!(state.session_count, 1);
    }

    #[test]
    fn save_and_load_session_state_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        let model = Model::new(PathBuf::from("/tmp/repo"));
        model.save_session_state(&path).unwrap();

        let loaded = Model::load_session_state(&path).unwrap();
        assert_eq!(loaded.display_mode, "tab");
        assert!(loaded.management_visible);
        assert_eq!(loaded.active_management_tab, "Branches");
        assert_eq!(loaded.session_count, 1);
    }

    #[test]
    fn save_session_state_with_grid_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        model.session_layout = SessionLayout::Grid;
        model.active_layer = ActiveLayer::Management;
        model.management_tab = ManagementTab::Settings;
        model.save_session_state(&path).unwrap();

        let loaded = Model::load_session_state(&path).unwrap();
        assert_eq!(loaded.display_mode, "grid");
        assert!(loaded.management_visible);
        assert_eq!(loaded.active_management_tab, "Settings");
    }

    #[test]
    fn load_session_state_missing_file_returns_none() {
        let result = Model::load_session_state(Path::new("/nonexistent/path/session.toml"));
        assert!(result.is_none());
    }

    #[test]
    fn model_new_initialization_defaults() {
        let model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        assert_eq!(model.active_layer, ActiveLayer::Initialization);
        assert!(model.initialization.is_some());
        let init = model.initialization.as_ref().unwrap();
        assert!(!init.bare_migration);
        assert!(init.url_input.is_empty());
    }

    #[test]
    fn model_new_initialization_bare_migration() {
        let model = Model::new_initialization(PathBuf::from("/tmp/bare"), true);
        assert_eq!(model.active_layer, ActiveLayer::Initialization);
        let init = model.initialization.as_ref().unwrap();
        assert!(init.bare_migration);
    }

    #[test]
    fn model_reset_transitions_to_management() {
        let mut model = Model::new_initialization(PathBuf::from("/tmp/empty"), false);
        assert_eq!(model.active_layer, ActiveLayer::Initialization);

        model.terminal_size = (120, 40);
        model.reset(PathBuf::from("/tmp/repo"));

        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert!(model.initialization.is_none());
        assert_eq!(model.repo_path, PathBuf::from("/tmp/repo"));
        // Terminal size is preserved
        assert_eq!(model.terminal_size, (120, 40));
    }

    #[test]
    fn save_session_state_tracks_session_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        // Add extra sessions
        model.sessions.push(SessionTab {
            id: "shell-1".to_string(),
            name: "Shell 2".to_string(),
            tab_type: SessionTabType::Shell,
            vt: VtState::new(24, 80),
        });
        model.sessions.push(SessionTab {
            id: "shell-2".to_string(),
            name: "Shell 3".to_string(),
            tab_type: SessionTabType::Shell,
            vt: VtState::new(24, 80),
        });
        model.save_session_state(&path).unwrap();

        let loaded = Model::load_session_state(&path).unwrap();
        assert_eq!(loaded.session_count, 3);
    }
}
