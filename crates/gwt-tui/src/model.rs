//! Model — central application state for the Elm Architecture.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::input::voice::VoiceInputState;
use crate::screens::branches::BranchesState;
use crate::screens::confirm::ConfirmState;
use crate::screens::docker_progress::DockerProgressState;
use crate::screens::git_view::GitViewState;
use crate::screens::issues::IssuesState;
use crate::screens::logs::LogsState;
use crate::screens::port_select::PortSelectState;
use crate::screens::pr_dashboard::PrDashboardState;
use crate::screens::profiles::ProfilesState;
use crate::screens::service_select::ServiceSelectState;
use crate::screens::settings::SettingsState;
use crate::screens::specs::SpecsState;
use crate::screens::versions::VersionsState;
use crate::screens::wizard::WizardState;

/// Which UI layer is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayer {
    /// Session panes (shell / agent terminals).
    Main,
    /// Management panel (branches, specs, issues, etc.).
    Management,
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
    Specs,
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
    pub const ALL: [ManagementTab; 9] = [
        ManagementTab::Branches,
        ManagementTab::Specs,
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
            Self::Specs => "Specs",
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
    /// Which layer has focus.
    pub active_layer: ActiveLayer,
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
    /// Specs screen state.
    pub(crate) specs: SpecsState,
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
    /// Voice input state.
    pub(crate) voice: VoiceInputState,
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

        Self {
            active_layer: ActiveLayer::Main,
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
            specs: SpecsState::default(),
            settings: SettingsState::default(),
            logs: LogsState::default(),
            versions: VersionsState::default(),
            wizard: None,
            docker_progress: None,
            service_select: None,
            port_select: None,
            confirm: ConfirmState::default(),
            voice: VoiceInputState::default(),
        }
    }

    /// Number of sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get the active session, if any.
    pub fn active_session_tab(&self) -> Option<&SessionTab> {
        self.sessions.get(self.active_session)
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
        let model = Model::new(PathBuf::from("/tmp/repo"));
        assert_eq!(model.active_layer, ActiveLayer::Main);
        assert_eq!(model.session_count(), 1);
        assert_eq!(model.active_session, 0);
        assert_eq!(model.session_layout, SessionLayout::Tab);
        assert_eq!(model.management_tab, ManagementTab::Branches);
        assert!(model.error_queue.is_empty());
        assert!(!model.quit);
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
    fn management_tab_all_has_nine_entries() {
        assert_eq!(ManagementTab::ALL.len(), 9);
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
        assert!(!loaded.management_visible);
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
