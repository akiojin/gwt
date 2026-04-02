//! Central Model: all application state lives here (Elm Architecture)

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use gwt_agent::types::AgentColor;
use gwt_terminal::PaneManager;

use crate::screens::branch_session_selector::BranchSessionSelectorState;
use crate::screens::branches::BranchListState;
use crate::screens::clone_wizard::CloneWizardState;
use crate::screens::confirm::ConfirmState;
use crate::screens::error::ErrorQueue;
use crate::screens::issues::IssuePanelState;
use crate::screens::speckit_wizard::SpecKitState;
use crate::screens::specs::SpecsState;
use crate::screens::versions::VersionsState;
use crate::screens::{LogsState, SettingsState};
use crate::widgets::progress_modal::ProgressState;

// ---------------------------------------------------------------------------
// Layer / Tab enums
// ---------------------------------------------------------------------------

/// Top-level layer: Main (sessions) vs Management (branches/issues/settings/logs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayer {
    /// Initialization screen (shown when no repo is detected)
    Initialization,
    Main,
    Management,
}

/// Management sub-tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementTab {
    Branches,
    Specs,
    Issues,
    Profiles,
    Versions,
    Settings,
    Logs,
}

impl ManagementTab {
    pub const ALL: [ManagementTab; 7] = [
        ManagementTab::Branches,
        ManagementTab::Specs,
        ManagementTab::Issues,
        ManagementTab::Profiles,
        ManagementTab::Settings,
        ManagementTab::Versions,
        ManagementTab::Logs,
    ];

    pub fn index(self) -> usize {
        match self {
            ManagementTab::Branches => 0,
            ManagementTab::Specs => 1,
            ManagementTab::Issues => 2,
            ManagementTab::Profiles => 3,
            ManagementTab::Versions => 4,
            ManagementTab::Settings => 5,
            ManagementTab::Logs => 6,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ManagementTab::Branches => "Branches",
            ManagementTab::Specs => "SPECs",
            ManagementTab::Issues => "Issues",
            ManagementTab::Profiles => "Profiles",
            ManagementTab::Versions => "Versions",
            ManagementTab::Settings => "Settings",
            ManagementTab::Logs => "Logs",
        }
    }

    pub fn visible_index(self) -> Option<usize> {
        Self::ALL.iter().position(|candidate| *candidate == self)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionLayoutMode {
    #[default]
    Grid,
    Maximized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPoint {
    pub row: u16,
    pub col: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalViewportState {
    pub follow_live: bool,
    pub scrollback: usize,
    pub max_scrollback: usize,
    pub selection_anchor: Option<SelectionPoint>,
    pub selection_focus: Option<SelectionPoint>,
    pub dragging: bool,
}

impl Default for TerminalViewportState {
    fn default() -> Self {
        Self {
            follow_live: true,
            scrollback: 0,
            max_scrollback: 0,
            selection_anchor: None,
            selection_focus: None,
            dragging: false,
        }
    }
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
// Error / overlay state (legacy types retained for backward compat)
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

/// Overlay mode for tracking which overlay is currently shown
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    None,
    Error,
    Confirm,
    Progress,
    CloneWizard,
    SpecKitWizard,
    BranchSessionSelector,
}

// WizardState is re-exported from screens::wizard
pub use crate::screens::wizard::WizardState;

// ---------------------------------------------------------------------------
// Background channel payloads
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct BranchListUpdate {
    pub branches: Vec<crate::screens::branches::BranchItem>,
}

#[derive(Debug)]
pub struct ManagementDataUpdate {
    pub issues: Vec<crate::screens::issues::IssueItem>,
    pub specs: Vec<crate::screens::specs::SpecItem>,
    pub versions: Vec<crate::screens::versions::VersionTag>,
    pub logs: Vec<crate::screens::logs::LogEntry>,
}

fn spawn_management_data_preload(repo_root: PathBuf) -> Receiver<ManagementDataUpdate> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let issues = crate::screens::issues::load_issues(&repo_root);
        let specs = crate::screens::specs::load_specs(&repo_root);
        let versions = crate::screens::versions::load_tags(&repo_root);
        let logs = crate::screens::logs::load_log_entries(&repo_root);
        let _ = tx.send(ManagementDataUpdate {
            issues,
            specs,
            versions,
            logs,
        });
    });
    rx
}

fn spawn_branch_list_enrichment(repo_root: PathBuf) -> Receiver<BranchListUpdate> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let branches = crate::screens::branches::load_branches_enriched(&repo_root);
        let _ = tx.send(BranchListUpdate { branches });
    });
    rx
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
    pub session_layout_mode: SessionLayoutMode,

    // Management tabs -- Management layer
    pub management_tab: ManagementTab,

    // Screen states for management tabs
    pub branches_state: BranchListState,
    pub issues_state: IssuePanelState,
    pub specs_state: SpecsState,
    pub settings_state: SettingsState,
    pub logs_state: LogsState,
    pub versions_state: VersionsState,

    // PTY management
    pub pane_manager: PaneManager,
    pub vt_parsers: HashMap<String, vt100::Parser>,
    pub pty_tx: Option<crate::event::PtyOutputSender>,
    pub terminal_viewports: HashMap<String, TerminalViewportState>,
    pub active_history_pane_id: Option<String>,
    pub active_history_parser: Option<vt100::Parser>,
    pub pending_resume_panes: HashSet<String>,

    // Overlay states
    pub overlay_mode: OverlayMode,
    pub error_queue: Vec<ErrorEntry>,
    pub error_queue_v2: ErrorQueue,
    pub progress: Option<ProgressState>,
    pub confirm: Option<ConfirmState>,
    pub branch_session_selector: Option<BranchSessionSelectorState>,
    pub wizard: Option<WizardState>,
    pub clone_wizard: Option<CloneWizardState>,
    pub speckit_wizard: SpecKitState,
    /// Pending Codex launch config waiting for hooks confirmation (SPEC-1786)
    pub pending_codex_launch: Option<crate::screens::wizard::WizardLaunchConfig>,

    // Background channels (for async operations)
    pub branch_list_rx: Option<Receiver<BranchListUpdate>>,
    pub management_data_rx: Option<Receiver<ManagementDataUpdate>>,

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
    /// Detects repo type and starts in Initialization layer if no repo is found,
    /// otherwise starts in Management layer with Branches tab active.
    pub fn new(repo_root: PathBuf) -> Self {
        let is_git_repo = gwt_git::Repository::open(&repo_root).is_ok();
        let active_layer = if is_git_repo {
            ActiveLayer::Management
        } else {
            ActiveLayer::Initialization
        };

        Self {
            active_layer,
            session_tabs: Vec::new(),
            active_session: 0,
            session_layout_mode: SessionLayoutMode::Grid,
            management_tab: ManagementTab::Branches,
            branches_state: BranchListState::new(),
            issues_state: IssuePanelState::new(),
            specs_state: SpecsState::new(),
            settings_state: SettingsState::new(),
            logs_state: LogsState::new(),
            versions_state: VersionsState::new(),
            pane_manager: PaneManager::new(80, 24),
            vt_parsers: HashMap::new(),
            pty_tx: None,
            terminal_viewports: HashMap::new(),
            active_history_pane_id: None,
            active_history_parser: None,
            pending_resume_panes: HashSet::new(),
            overlay_mode: OverlayMode::None,
            error_queue: Vec::new(),
            error_queue_v2: ErrorQueue::new(),
            progress: None,
            confirm: None,
            branch_session_selector: None,
            wizard: None,
            clone_wizard: None,
            speckit_wizard: SpecKitState::new(),
            pending_codex_launch: None,
            branch_list_rx: None,
            management_data_rx: None,
            should_quit: false,
            repo_root,
            terminal_rows: 24,
            terminal_cols: 80,
            last_ctrl_c: None,
            tick_count: 0,
        }
    }

    /// Reset the model for a new repository root.
    /// Clears session state, reloads all management screen data,
    /// and transitions to the branch-first management entry.
    pub fn reset(&mut self, new_repo_root: std::path::PathBuf) {
        self.repo_root = new_repo_root;
        self.session_tabs.clear();
        self.active_session = 0;
        self.session_layout_mode = SessionLayoutMode::Grid;
        self.active_layer = ActiveLayer::Management;
        self.management_tab = ManagementTab::Branches;
        self.overlay_mode = OverlayMode::None;
        self.branch_session_selector = None;
        self.clone_wizard = None;
        self.load_all_data();
    }

    /// Load all management screen data from the current repo_root.
    pub fn load_all_data(&mut self) {
        let repo_root = self.repo_root.clone();
        self.branches_state.branches = crate::screens::branches::load_branches(&repo_root);
        self.settings_state.load_settings();
        self.sync_branch_session_counts();
        self.branch_list_rx = Some(spawn_branch_list_enrichment(repo_root.clone()));
        self.management_data_rx = Some(spawn_management_data_preload(repo_root));
    }

    // ---- Session tab helpers ------------------------------------------------

    /// Add a new session tab and switch to it.
    pub fn add_session(&mut self, tab: SessionTab) {
        self.clear_active_history_view();
        self.session_tabs.push(tab);
        self.active_session = self.session_tabs.len() - 1;
        self.session_layout_mode = SessionLayoutMode::Grid;
        self.active_layer = ActiveLayer::Main;
        self.sync_branch_session_counts();
    }

    /// Close the session at `index`. Returns the removed tab, or `None`.
    pub fn close_session(&mut self, index: usize) -> Option<SessionTab> {
        if index >= self.session_tabs.len() {
            return None;
        }
        let tab = self.session_tabs.remove(index);
        if self.active_history_pane_id.as_deref() == Some(tab.pane_id.as_str()) {
            self.clear_active_history_view();
        }
        self.terminal_viewports.remove(&tab.pane_id);
        self.vt_parsers.remove(&tab.pane_id);
        self.pending_resume_panes.remove(&tab.pane_id);
        let _ = self.pane_manager.close_pane(&tab.pane_id);
        if self.session_tabs.is_empty() {
            self.active_session = 0;
            self.active_layer = ActiveLayer::Management;
        } else if self.active_session >= self.session_tabs.len() {
            self.active_session = self.session_tabs.len() - 1;
        }
        self.sync_branch_session_counts();
        Some(tab)
    }

    /// Close the currently active session.
    pub fn close_active_session(&mut self) -> Option<SessionTab> {
        if self.session_tabs.is_empty() {
            return None;
        }
        self.close_session(self.active_session)
    }

    pub fn close_session_by_pane_id(&mut self, pane_id: &str) -> Option<SessionTab> {
        let index = self
            .session_tabs
            .iter()
            .position(|tab| tab.pane_id == pane_id)?;
        self.close_session(index)
    }

    pub fn running_session_count(&self) -> usize {
        self.session_tabs
            .iter()
            .filter(|t| matches!(t.status, SessionStatus::Running))
            .count()
    }

    pub fn running_agent_count(&self) -> usize {
        self.session_tabs
            .iter()
            .filter(|t| {
                t.tab_type == SessionTabType::Agent && matches!(t.status, SessionStatus::Running)
            })
            .count()
    }

    /// Switch to next session (wraps).
    pub fn next_session(&mut self) {
        if self.session_tabs.is_empty() {
            return;
        }
        self.clear_active_history_view();
        self.active_session = (self.active_session + 1) % self.session_tabs.len();
    }

    /// Switch to previous session (wraps).
    pub fn prev_session(&mut self) {
        if self.session_tabs.is_empty() {
            return;
        }
        self.clear_active_history_view();
        self.active_session = if self.active_session == 0 {
            self.session_tabs.len() - 1
        } else {
            self.active_session - 1
        };
    }

    /// Switch to session by 0-based index.
    pub fn switch_session(&mut self, index: usize) {
        if index < self.session_tabs.len() {
            self.clear_active_history_view();
            self.active_session = index;
        }
    }

    pub fn toggle_session_layout_mode(&mut self) {
        if self.session_tabs.is_empty() {
            return;
        }
        self.session_layout_mode = match self.session_layout_mode {
            SessionLayoutMode::Grid => SessionLayoutMode::Maximized,
            SessionLayoutMode::Maximized => SessionLayoutMode::Grid,
        };
    }

    // ---- Layer helpers -------------------------------------------------------

    /// Toggle between Main and Management layers.
    pub fn toggle_layer(&mut self) {
        self.active_layer = match self.active_layer {
            ActiveLayer::Initialization => ActiveLayer::Initialization, // Blocked during init
            ActiveLayer::Main => {
                self.clear_active_history_view();
                ActiveLayer::Management
            }
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
        use gwt_terminal::PaneStatus;

        self.tick_count += 1;
        // Poll branch list updates
        let mut branch_updates = Vec::new();
        if let Some(ref rx) = self.branch_list_rx {
            while let Ok(update) = rx.try_recv() {
                branch_updates.push(update);
            }
        }
        for update in branch_updates {
            self.branches_state.set_branches(update.branches);
            self.sync_branch_session_counts();
        }

        let mut management_updates = Vec::new();
        if let Some(ref rx) = self.management_data_rx {
            while let Ok(update) = rx.try_recv() {
                management_updates.push(update);
            }
        }
        for update in management_updates {
            self.issues_state.issues = update.issues;
            self.specs_state.specs = update.specs;
            self.versions_state.tags = update.versions;
            self.logs_state.entries = update.logs;
        }

        // Collect pane IDs from session tabs for status polling
        let pane_ids: Vec<String> = self.session_tabs.iter().map(|t| t.pane_id.clone()).collect();
        let mut session_status_updates = Vec::new();
        for pane_id in &pane_ids {
            if let Some(pane) = self.pane_manager.get_pane_mut(pane_id) {
                let status = match pane.check_status() {
                    Ok(status) => status.clone(),
                    Err(err) => {
                        let message = err.to_string();
                        pane.mark_error(message.clone());
                        PaneStatus::Error(message)
                    }
                };
                session_status_updates.push((pane_id.clone(), map_pane_status(&status)));
            }
        }

        for (pane_id, status) in session_status_updates {
            if let Some(tab) = self
                .session_tabs
                .iter_mut()
                .find(|tab| tab.pane_id == pane_id)
            {
                tab.status = status;
            }
        }
    }

    pub fn terminal_viewport_mut(&mut self, pane_id: &str) -> &mut TerminalViewportState {
        self.terminal_viewports
            .entry(pane_id.to_string())
            .or_default()
    }

    pub fn terminal_viewport(&self, pane_id: &str) -> Option<&TerminalViewportState> {
        self.terminal_viewports.get(pane_id)
    }

    pub fn clear_active_history_view(&mut self) {
        self.active_history_pane_id = None;
        self.active_history_parser = None;
    }

    pub fn sync_branch_session_counts(&mut self) {
        for branch in &mut self.branches_state.branches {
            branch.session_count = 0;
            branch.running_session_count = 0;
            branch.stopped_session_count = 0;
        }

        for tab in &self.session_tabs {
            let Some(tab_branch) = tab.branch.as_deref() else {
                continue;
            };
            let normalized_tab = normalize_branch_name(tab_branch);
            for branch in &mut self.branches_state.branches {
                if normalize_branch_name(&branch.name) == normalized_tab {
                    branch.session_count += 1;
                    match tab.status {
                        SessionStatus::Running => branch.running_session_count += 1,
                        SessionStatus::Completed(_) | SessionStatus::Error(_) => {
                            branch.stopped_session_count += 1
                        }
                    }
                }
            }
        }
    }
}

fn normalize_branch_name(name: &str) -> &str {
    if let Some(stripped) = name.strip_prefix("remotes/") {
        if let Some((_, rest)) = stripped.split_once('/') {
            return rest;
        }
        return stripped;
    }

    if let Some(stripped) = name.strip_prefix("origin/") {
        return stripped;
    }
    if let Some(stripped) = name.strip_prefix("upstream/") {
        return stripped;
    }

    name
}

fn map_pane_status(status: &gwt_terminal::PaneStatus) -> SessionStatus {
    match status {
        gwt_terminal::PaneStatus::Running => SessionStatus::Running,
        gwt_terminal::PaneStatus::Completed(code) => SessionStatus::Completed(*code),
        gwt_terminal::PaneStatus::Error(message) => SessionStatus::Error(message.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, path::PathBuf, thread, time::Duration};

    fn test_model() -> Model {
        let mut m = Model::new(PathBuf::from("/tmp/test-repo"));
        m.active_layer = ActiveLayer::Management; // Force Management for tests
        m
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

    /// Launch a test pane and return its actual pane ID (generated by PaneManager).
    fn attach_test_pane(model: &mut Model, _hint: &str, command: &str, args: &[&str]) -> String {
        let config = gwt_terminal::manager::LaunchConfig {
            command: command.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env: HashMap::new(),
            cwd: Some(std::env::temp_dir()),
        };
        model
            .pane_manager
            .launch_agent(config)
            .expect("failed to launch test pane")
    }

    fn wait_for_session_count(model: &mut Model, expected_sessions: usize) {
        for _ in 0..50 {
            model.apply_background_updates();
            if model.session_tabs.len() == expected_sessions {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "expected {} sessions after auto-close, got {}",
            expected_sessions,
            model.session_tabs.len()
        );
    }

    fn wait_for_session_status(model: &mut Model, pane_id: &str, expected: SessionStatus) {
        for _ in 0..50 {
            model.apply_background_updates();
            if model
                .session_tabs
                .iter()
                .find(|tab| tab.pane_id == pane_id)
                .is_some_and(|tab| tab.status == expected)
            {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("expected {pane_id} to reach status {:?}", expected);
    }

    fn wait_for_management_data(model: &mut Model, expected_specs: usize, expected_issues: usize) {
        for _ in 0..50 {
            model.apply_background_updates();
            if model.specs_state.specs.len() == expected_specs
                && model.issues_state.issues.len() == expected_issues
            {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "expected specs={}, issues={}, got specs={}, issues={}",
            expected_specs,
            expected_issues,
            model.specs_state.specs.len(),
            model.issues_state.issues.len()
        );
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
    fn close_session_removes_matching_vt_parser() {
        let mut m = test_model();
        m.add_session(test_session("s1", SessionTabType::Shell));
        m.vt_parsers
            .insert("pane-s1".to_string(), vt100::Parser::new(24, 80, 0));

        m.close_active_session();

        assert!(!m.vt_parsers.contains_key("pane-s1"));
    }

    #[test]
    fn apply_background_updates_keeps_completed_agent_session_visible() {
        let mut m = test_model();
        let pane_id = attach_test_pane(&mut m, "agent", "/usr/bin/true", &[]);
        m.add_session(SessionTab {
            pane_id: pane_id.clone(),
            name: "agent".to_string(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: Some("feature/test".to_string()),
            spec_id: None,
        });

        wait_for_session_count(&mut m, 1);
        wait_for_session_status(&mut m, &pane_id, SessionStatus::Completed(0));

        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.session_tabs.len(), 1);
        assert_eq!(m.session_tabs[0].status, SessionStatus::Completed(0));
    }

    #[test]
    fn apply_background_updates_keeps_completed_shell_session_visible() {
        let mut m = test_model();
        let pane_id = attach_test_pane(&mut m, "shell", "/usr/bin/true", &[]);
        m.add_session(SessionTab {
            pane_id: pane_id.clone(),
            name: "shell".to_string(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        });

        wait_for_session_count(&mut m, 1);
        wait_for_session_status(&mut m, &pane_id, SessionStatus::Completed(0));

        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.session_tabs.len(), 1);
        assert_eq!(m.session_tabs[0].status, SessionStatus::Completed(0));
    }

    #[test]
    fn apply_background_updates_keeps_failed_session_visible() {
        let mut m = test_model();
        let pane_id = attach_test_pane(&mut m, "failed", "/usr/bin/false", &[]);
        m.add_session(SessionTab {
            pane_id: pane_id.clone(),
            name: "shell".to_string(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        });

        wait_for_session_count(&mut m, 1);
        wait_for_session_status(&mut m, &pane_id, SessionStatus::Completed(1));

        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.session_tabs.len(), 1);
        assert_eq!(m.session_tabs[0].status, SessionStatus::Completed(1));
    }

    #[test]
    fn apply_background_updates_keeps_completed_session_focused() {
        let mut m = test_model();
        let slow_id = attach_test_pane(&mut m, "slow", "/bin/sleep", &["60"]);
        m.add_session(SessionTab {
            pane_id: slow_id.clone(),
            name: "shell".to_string(),
            tab_type: SessionTabType::Shell,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: None,
            spec_id: None,
        });
        let done_id = attach_test_pane(&mut m, "done", "/usr/bin/true", &[]);
        m.add_session(SessionTab {
            pane_id: done_id.clone(),
            name: "agent".to_string(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: Some("feature/test".to_string()),
            spec_id: None,
        });

        wait_for_session_count(&mut m, 2);
        wait_for_session_status(&mut m, &done_id, SessionStatus::Completed(0));

        assert_eq!(m.active_layer, ActiveLayer::Main);
        assert_eq!(m.active_session, 1);
        assert_eq!(m.session_tabs[0].pane_id, slow_id);
        assert_eq!(m.session_tabs[0].status, SessionStatus::Running);
        assert_eq!(m.session_tabs[1].pane_id, done_id);
        assert_eq!(m.session_tabs[1].status, SessionStatus::Completed(0));

        let _ = m.close_active_session();
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
        assert_eq!(ManagementTab::Specs.index(), 1);
        assert_eq!(ManagementTab::Issues.index(), 2);
        assert_eq!(ManagementTab::Profiles.index(), 3);
        assert_eq!(ManagementTab::Versions.index(), 4);
        assert_eq!(ManagementTab::Settings.index(), 5);
        assert_eq!(ManagementTab::Logs.index(), 6);
        assert_eq!(ManagementTab::ALL[1].label(), "SPECs");
        assert_eq!(ManagementTab::ALL[2].label(), "Issues");
        assert_eq!(ManagementTab::ALL[3].label(), "Profiles");
        assert_eq!(ManagementTab::ALL[4].label(), "Settings");
        assert_eq!(ManagementTab::ALL[5].label(), "Versions");
        assert_eq!(ManagementTab::ALL[6].label(), "Logs");
        assert_eq!(ManagementTab::ALL.len(), 7);
    }

    #[test]
    fn tick_increments_count() {
        let mut m = test_model();
        m.apply_background_updates();
        assert_eq!(m.tick_count, 1);
        m.apply_background_updates();
        assert_eq!(m.tick_count, 2);
    }

    #[test]
    fn reset_returns_to_branch_first_management_entry() {
        let mut model = test_model();
        model.management_tab = ManagementTab::Issues;
        model.add_session(test_session("s1", SessionTabType::Shell));

        let repo = tempfile::tempdir().unwrap();

        model.reset(repo.path().to_path_buf());

        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.management_tab, ManagementTab::Branches);
        assert!(model.session_tabs.is_empty());
        assert_eq!(model.session_layout_mode, SessionLayoutMode::Grid);
    }

    #[test]
    fn sync_branch_session_counts_tracks_running_and_stopped_sessions() {
        let mut model = test_model();
        model.branches_state.branches = vec![crate::screens::branches::BranchItem {
            name: "feature/demo".to_string(),
            is_current: false,
            has_worktree: true,
            worktree_path: Some("/tmp/feature-demo".to_string()),
            session_count: 0,
            running_session_count: 0,
            stopped_session_count: 0,
            worktree_indicator: 'w',
            has_changes: false,
            has_unpushed: false,
            is_protected: false,
            last_tool_usage: None,
            last_tool_id: None,
            quick_start_available: false,
            linked_issue_number: None,
            linked_issue_state: None,
            pr_title: None,
            pr_number: None,
            pr_state: None,
            safety_status: crate::screens::branches::SafetyStatus::Safe,
            is_remote: false,
            last_commit_timestamp: None,
        }];
        model.add_session(SessionTab {
            pane_id: "pane-running".to_string(),
            name: "running".to_string(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Green,
            status: SessionStatus::Running,
            branch: Some("feature/demo".to_string()),
            spec_id: None,
        });
        model.add_session(SessionTab {
            pane_id: "pane-done".to_string(),
            name: "done".to_string(),
            tab_type: SessionTabType::Agent,
            color: AgentColor::Green,
            status: SessionStatus::Completed(0),
            branch: Some("feature/demo".to_string()),
            spec_id: None,
        });

        model.sync_branch_session_counts();

        let branch = &model.branches_state.branches[0];
        assert_eq!(branch.session_count, 2);
        assert_eq!(branch.running_session_count, 1);
        assert_eq!(branch.stopped_session_count, 1);
    }

    #[test]
    fn apply_background_updates_applies_management_data_preload() {
        let mut model = test_model();
        let (tx, rx) = std::sync::mpsc::channel();
        model.management_data_rx = Some(rx);

        tx.send(ManagementDataUpdate {
            issues: vec![crate::screens::issues::IssueItem {
                number: 7,
                title: "Loaded issue".to_string(),
                state: "OPEN".to_string(),
                labels: vec!["bug".to_string()],
            }],
            specs: vec![crate::screens::specs::SpecItem {
                dir_name: "SPEC-7".to_string(),
                id: "7".to_string(),
                title: "Loaded spec".to_string(),
                status: "open".to_string(),
                phase: "draft".to_string(),
                branches: vec![],
            }],
            versions: vec![crate::screens::versions::VersionTag {
                id: "v1".to_string(),
                label: "v1.0.0".to_string(),
                range_from: None,
                range_to: "v1.0.0".to_string(),
                commit_count: 1,
                summary_preview: "initial".to_string(),
            }],
            logs: vec![crate::screens::logs::LogEntry {
                timestamp: "2026-04-02T00:00:00Z".to_string(),
                level: "INFO".to_string(),
                message: "loaded".to_string(),
                target: "gwt".to_string(),
                category: Some("ui".to_string()),
                event: Some("preload".to_string()),
                result: Some("success".to_string()),
                workspace: Some("default".to_string()),
                error_code: None,
                error_detail: None,
                extra: std::collections::BTreeMap::new(),
            }],
        })
        .unwrap();

        model.apply_background_updates();

        assert_eq!(model.issues_state.issues.len(), 1);
        assert_eq!(model.specs_state.specs.len(), 1);
        assert_eq!(model.versions_state.tags.len(), 1);
        assert_eq!(model.logs_state.entries.len(), 1);
    }

    #[test]
    fn apply_background_updates_applies_branch_list_enrichment() {
        let mut model = test_model();
        let (tx, rx) = std::sync::mpsc::channel();
        model.branch_list_rx = Some(rx);

        tx.send(BranchListUpdate {
            branches: vec![crate::screens::branches::BranchItem {
                name: "feature/demo".to_string(),
                is_current: false,
                has_worktree: true,
                worktree_path: Some("/tmp/demo".to_string()),
                session_count: 0,
                running_session_count: 0,
                stopped_session_count: 0,
                worktree_indicator: 'w',
                has_changes: false,
                has_unpushed: true,
                is_protected: false,
                last_tool_usage: None,
                last_tool_id: None,
                quick_start_available: false,
                linked_issue_number: None,
                linked_issue_state: None,
                pr_title: Some("Demo PR".to_string()),
                pr_number: Some(7),
                pr_state: Some("open".to_string()),
                safety_status: crate::screens::branches::SafetyStatus::Warning,
                is_remote: false,
                last_commit_timestamp: None,
            }],
        })
        .unwrap();

        model.apply_background_updates();

        assert_eq!(model.branches_state.branches.len(), 1);
        let branch = &model.branches_state.branches[0];
        assert_eq!(branch.pr_number, Some(7));
        assert_eq!(branch.pr_title.as_deref(), Some("Demo PR"));
        assert_eq!(
            branch.safety_status,
            crate::screens::branches::SafetyStatus::Warning
        );
        assert_eq!(branch.worktree_indicator, 'w');
    }
}
