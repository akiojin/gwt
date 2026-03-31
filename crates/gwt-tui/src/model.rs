//! Central Model: all application state lives here (Elm Architecture)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use gwt_core::terminal::manager::PaneManager;
use gwt_core::terminal::AgentColor;

use crate::screens::branches::BranchListState;
use crate::screens::issues::IssuePanelState;

// ---------------------------------------------------------------------------
// Layer / Tab enums
// ---------------------------------------------------------------------------

/// Top-level layer: Main (sessions) vs Management (branches/issues/settings/logs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayer {
    Main,
    Management,
}

/// Management sub-tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementTab {
    Branches,
    Issues,
    Settings,
    Logs,
}

impl ManagementTab {
    pub const ALL: [ManagementTab; 4] = [
        ManagementTab::Branches,
        ManagementTab::Issues,
        ManagementTab::Settings,
        ManagementTab::Logs,
    ];

    pub fn index(self) -> usize {
        match self {
            ManagementTab::Branches => 0,
            ManagementTab::Issues => 1,
            ManagementTab::Settings => 2,
            ManagementTab::Logs => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ManagementTab::Branches => "Branches",
            ManagementTab::Issues => "Issues",
            ManagementTab::Settings => "Settings",
            ManagementTab::Logs => "Logs",
        }
    }
}

// ---------------------------------------------------------------------------
// Session tab types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTabType {
    Shell,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Completed(i32),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SessionTab {
    pub pane_id: String,
    pub name: String,
    pub tab_type: SessionTabType,
    pub color: AgentColor,
    pub status: SessionStatus,
    pub branch: Option<String>,
    pub spec_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Error / overlay state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Critical,
    Minor,
}

#[derive(Debug, Clone)]
pub struct ErrorEntry {
    pub message: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone)]
pub struct ProgressState {
    pub title: String,
    pub detail: Option<String>,
    pub percent: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub cancel_label: String,
}

/// Placeholder for Phase 3 wizard
#[derive(Debug, Clone)]
pub struct WizardState {
    _private: (),
}

// ---------------------------------------------------------------------------
// Background channel payloads
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct BranchListUpdate {
    pub branches: Vec<String>,
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Central application state (Elm Architecture Model).
pub struct Model {
    // Layer management (2-layer tab structure)
    pub active_layer: ActiveLayer,

    // Session tabs (Agent + Shell) -- Main layer
    pub session_tabs: Vec<SessionTab>,
    pub active_session: usize,

    // Management tabs -- Management layer
    pub management_tab: ManagementTab,

    // Screen states for management tabs
    pub branches_state: BranchListState,
    pub issues_state: IssuePanelState,

    // PTY management
    pub pane_manager: PaneManager,
    pub vt_parsers: HashMap<String, vt100::Parser>,

    // Overlay states
    pub wizard: Option<WizardState>,
    pub error_queue: Vec<ErrorEntry>,
    pub progress: Option<ProgressState>,
    pub confirm: Option<ConfirmState>,

    // Background channels (for async operations)
    pub branch_list_rx: Option<Receiver<BranchListUpdate>>,

    // App lifecycle
    pub should_quit: bool,
    pub repo_root: PathBuf,
    pub terminal_rows: u16,
    pub terminal_cols: u16,
    pub last_ctrl_c: Option<Instant>,
    pub tick_count: u64,
}

impl Model {
    /// Create a new Model with default state.
    /// Starts in Management layer with Branches tab active.
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            active_layer: ActiveLayer::Management,
            session_tabs: Vec::new(),
            active_session: 0,
            management_tab: ManagementTab::Branches,
            branches_state: BranchListState::new(),
            issues_state: IssuePanelState::new(),
            pane_manager: PaneManager::new(),
            vt_parsers: HashMap::new(),
            wizard: None,
            error_queue: Vec::new(),
            progress: None,
            confirm: None,
            branch_list_rx: None,
            should_quit: false,
            repo_root,
            terminal_rows: 24,
            terminal_cols: 80,
            last_ctrl_c: None,
            tick_count: 0,
        }
    }

    // ---- Session tab helpers ------------------------------------------------

    /// Add a new session tab and switch to it.
    pub fn add_session(&mut self, tab: SessionTab) {
        self.session_tabs.push(tab);
        self.active_session = self.session_tabs.len() - 1;
        self.active_layer = ActiveLayer::Main;
    }

    /// Close the session at `index`. Returns the removed tab, or `None`.
    pub fn close_session(&mut self, index: usize) -> Option<SessionTab> {
        if index >= self.session_tabs.len() {
            return None;
        }
        let tab = self.session_tabs.remove(index);
        let _ = self.pane_manager.close_pane(index);
        if self.session_tabs.is_empty() {
            self.active_session = 0;
            self.active_layer = ActiveLayer::Management;
        } else if self.active_session >= self.session_tabs.len() {
            self.active_session = self.session_tabs.len() - 1;
        }
        Some(tab)
    }

    /// Close the currently active session.
    pub fn close_active_session(&mut self) -> Option<SessionTab> {
        if self.session_tabs.is_empty() {
            return None;
        }
        self.close_session(self.active_session)
    }

    /// Switch to next session (wraps).
    pub fn next_session(&mut self) {
        if self.session_tabs.is_empty() {
            return;
        }
        self.active_session = (self.active_session + 1) % self.session_tabs.len();
    }

    /// Switch to previous session (wraps).
    pub fn prev_session(&mut self) {
        if self.session_tabs.is_empty() {
            return;
        }
        self.active_session = if self.active_session == 0 {
            self.session_tabs.len() - 1
        } else {
            self.active_session - 1
        };
    }

    /// Switch to session by 0-based index.
    pub fn switch_session(&mut self, index: usize) {
        if index < self.session_tabs.len() {
            self.active_session = index;
        }
    }

    // ---- Layer helpers -------------------------------------------------------

    /// Toggle between Main and Management layers.
    pub fn toggle_layer(&mut self) {
        self.active_layer = match self.active_layer {
            ActiveLayer::Main => ActiveLayer::Management,
            ActiveLayer::Management => {
                if self.session_tabs.is_empty() {
                    // Stay in Management if no sessions exist
                    ActiveLayer::Management
                } else {
                    ActiveLayer::Main
                }
            }
        };
    }

    // ---- Error helpers -------------------------------------------------------

    pub fn push_error(&mut self, entry: ErrorEntry) {
        self.error_queue.push(entry);
    }

    pub fn dismiss_error(&mut self) {
        if !self.error_queue.is_empty() {
            self.error_queue.remove(0);
        }
    }

    // ---- Ctrl+C handling -----------------------------------------------------

    /// Handle Ctrl+C press. Returns true if app should quit (double-tap).
    pub fn handle_ctrl_c(&mut self) -> bool {
        let now = Instant::now();
        if let Some(last) = self.last_ctrl_c {
            if now.duration_since(last) < std::time::Duration::from_millis(500) {
                self.should_quit = true;
                return true;
            }
        }
        self.last_ctrl_c = Some(now);
        false
    }

    // ---- Background update polling -------------------------------------------

    pub fn apply_background_updates(&mut self) {
        self.tick_count += 1;
        // Poll branch list updates
        if let Some(ref rx) = self.branch_list_rx {
            while let Ok(_update) = rx.try_recv() {
                // Phase 2: apply branch list data to screens
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_model() -> Model {
        Model::new(PathBuf::from("/tmp/test-repo"))
    }

    fn test_session(name: &str, tab_type: SessionTabType) -> SessionTab {
        SessionTab {
            pane_id: format!("pane-{name}"),
            name: name.to_string(),
            tab_type,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        }
    }

    #[test]
    fn initial_state_starts_in_management_branches() {
        let m = test_model();
        assert_eq!(m.active_layer, ActiveLayer::Management);
        assert_eq!(m.management_tab, ManagementTab::Branches);
        assert!(m.session_tabs.is_empty());
        assert!(!m.should_quit);
        assert_eq!(m.tick_count, 0);
    }

    #[test]
    fn toggle_layer_stays_management_when_no_sessions() {
        let mut m = test_model();
        m.toggle_layer();
        assert_eq!(m.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn toggle_layer_switches_when_sessions_exist() {
        let mut m = test_model();
        m.add_session(test_session("shell-1", SessionTabType::Shell));
        // add_session switches to Main automatically
        assert_eq!(m.active_layer, ActiveLayer::Main);
        m.toggle_layer();
        assert_eq!(m.active_layer, ActiveLayer::Management);
        m.toggle_layer();
        assert_eq!(m.active_layer, ActiveLayer::Main);
    }

    #[test]
    fn add_session_switches_to_main_layer() {
        let mut m = test_model();
        m.add_session(test_session("agent-1", SessionTabType::Agent));
        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.active_session, 0);
        assert_eq!(m.session_tabs.len(), 1);
    }

    #[test]
    fn session_next_prev_wraps() {
        let mut m = test_model();
        m.add_session(test_session("s1", SessionTabType::Shell));
        m.add_session(test_session("s2", SessionTabType::Agent));
        m.add_session(test_session("s3", SessionTabType::Shell));
        assert_eq!(m.active_session, 2);

        m.next_session();
        assert_eq!(m.active_session, 0);

        m.prev_session();
        assert_eq!(m.active_session, 2);
    }

    #[test]
    fn switch_session_by_index() {
        let mut m = test_model();
        m.add_session(test_session("s1", SessionTabType::Shell));
        m.add_session(test_session("s2", SessionTabType::Agent));
        m.switch_session(0);
        assert_eq!(m.active_session, 0);
        // Out of range does nothing
        m.switch_session(99);
        assert_eq!(m.active_session, 0);
    }

    #[test]
    fn close_session_returns_to_management_when_empty() {
        let mut m = test_model();
        m.add_session(test_session("s1", SessionTabType::Shell));
        assert_eq!(m.active_layer, ActiveLayer::Main);
        m.close_active_session();
        assert!(m.session_tabs.is_empty());
        assert_eq!(m.active_layer, ActiveLayer::Management);
    }

    #[test]
    fn close_session_adjusts_active_index() {
        let mut m = test_model();
        m.add_session(test_session("s1", SessionTabType::Shell));
        m.add_session(test_session("s2", SessionTabType::Shell));
        m.add_session(test_session("s3", SessionTabType::Shell));
        // active = 2 (last added)
        m.close_session(2);
        assert_eq!(m.active_session, 1);
        assert_eq!(m.session_tabs.len(), 2);
    }

    #[test]
    fn ctrl_c_double_tap_quits() {
        let mut m = test_model();
        assert!(!m.handle_ctrl_c());
        // Immediate second tap
        assert!(m.handle_ctrl_c());
        assert!(m.should_quit);
    }

    #[test]
    fn error_queue_push_dismiss() {
        let mut m = test_model();
        assert!(m.error_queue.is_empty());
        m.push_error(ErrorEntry {
            message: "test error".into(),
            severity: ErrorSeverity::Minor,
        });
        assert_eq!(m.error_queue.len(), 1);
        m.dismiss_error();
        assert!(m.error_queue.is_empty());
        // Dismiss on empty is safe
        m.dismiss_error();
    }

    #[test]
    fn management_tab_metadata() {
        assert_eq!(ManagementTab::Branches.index(), 0);
        assert_eq!(ManagementTab::Logs.label(), "Logs");
        assert_eq!(ManagementTab::ALL.len(), 4);
    }

    #[test]
    fn tick_increments_count() {
        let mut m = test_model();
        m.apply_background_updates();
        assert_eq!(m.tick_count, 1);
        m.apply_background_updates();
        assert_eq!(m.tick_count, 2);
    }
}
