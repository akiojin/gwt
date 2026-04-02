//! Model — central application state for the Elm Architecture.

use std::path::PathBuf;

use crate::screens::branches::BranchesState;
use crate::screens::profiles::ProfilesState;

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
        ManagementTab::Specs,
        ManagementTab::Issues,
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
    pub sessions: Vec<SessionTab>,
    /// Index of the active session.
    pub active_session: usize,
    /// Session layout mode.
    pub session_layout: SessionLayout,
    /// Active management tab.
    pub management_tab: ManagementTab,
    /// Whether the management panel is visible.
    pub management_visible: bool,
    /// Error queue (shown as overlays).
    pub error_queue: Vec<String>,
    /// Whether the app should quit.
    pub quit: bool,
    /// Repository path.
    pub repo_path: PathBuf,
    /// Terminal size.
    pub terminal_size: (u16, u16),
    /// Branches screen state.
    pub branches: BranchesState,
    /// Profiles screen state.
    pub profiles: ProfilesState,
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
            management_visible: false,
            error_queue: Vec::new(),
            quit: false,
            repo_path,
            terminal_size: (80, 24),
            branches: BranchesState::default(),
            profiles: ProfilesState::default(),
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
        assert!(!model.management_visible);
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
    fn management_tab_all_has_eight_entries() {
        assert_eq!(ManagementTab::ALL.len(), 8);
    }

    #[test]
    fn vt_state_dimensions() {
        let vt = VtState::new(40, 120);
        assert_eq!(vt.rows(), 40);
        assert_eq!(vt.cols(), 120);
    }
}
