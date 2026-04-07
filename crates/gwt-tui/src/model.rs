//! Model — central application state for the Elm Architecture.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use gwt_config::VoiceConfig;
use gwt_voice::{NoOpVoiceBackend, Qwen3AsrRecorder, VoiceBackend, VoiceSession};
use serde::{Deserialize, Serialize};

use crate::input::voice::VoiceInputState;
use crate::screens::branches::{BranchDetailLoadResult, BranchesState};
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

type BoxedVoiceBackend = Box<dyn VoiceBackend + Send>;

fn build_voice_backend(config: &VoiceConfig) -> BoxedVoiceBackend {
    if config.enabled && config.model_path.is_some() {
        Box::new(Qwen3AsrRecorder::new())
    } else {
        Box::new(NoOpVoiceBackend::new())
    }
}

/// Runtime voice session state shared across start/stop messages.
#[derive(Default)]
pub(crate) struct VoiceRuntimeState {
    config: VoiceConfig,
    session: Option<VoiceSession<BoxedVoiceBackend>>,
}

impl VoiceRuntimeState {
    pub(crate) fn configure(&mut self, config: &VoiceConfig) {
        if self.session.is_none() {
            self.config = config.clone();
        }
    }

    pub(crate) fn start_recording(&mut self) -> Result<(), String> {
        if self.session.is_none() {
            self.session = Some(VoiceSession::new(build_voice_backend(&self.config)));
        }

        let result = self
            .session
            .as_mut()
            .expect("voice session initialized")
            .start_recording()
            .map_err(|err| err.to_string());

        if result.is_err() {
            self.session = None;
        }

        result
    }

    pub(crate) fn stop_and_transcribe(&mut self) -> Result<String, String> {
        let Some(mut session) = self.session.take() else {
            return Err("Not currently recording".to_string());
        };

        session.stop_and_transcribe().map_err(|err| err.to_string())
    }

    pub(crate) fn reset(&mut self) {
        self.session = None;
    }
}

impl std::fmt::Debug for VoiceRuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceRuntimeState")
            .field("config", &self.config)
            .field("has_session", &self.session.is_some())
            .finish()
    }
}

/// Which UI layer is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveLayer {
    /// Initialization screen (no repo detected — clone wizard or bare migration).
    Initialization,
    /// Session panes (shell / agent terminals).
    Main,
    /// Management panel (branches, issues, PRs, settings, etc.).
    Management,
}

/// Which pane currently owns keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusPane {
    /// Tab content area (↑↓ navigates list, Left/Right switches tabs).
    #[default]
    TabContent,
    /// Branch detail panel (←→ sections, ↑↓ actions).
    BranchDetail,
    /// Terminal PTY (all keys forwarded).
    Terminal,
}

impl FocusPane {
    const ALL: [FocusPane; 3] = [
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
        Self::ALL[if idx == 0 {
            Self::ALL.len() - 1
        } else {
            idx - 1
        }]
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

    /// Next tab (wraps around).
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Previous tab (wraps around).
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[if idx == 0 {
            Self::ALL.len() - 1
        } else {
            idx - 1
        }]
    }
}

/// Type of a session tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTabType {
    Shell,
    Agent { agent_id: String, color: AgentColor },
}

impl SessionTabType {
    /// Unicode icon for this session type.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Shell => crate::theme::icon::SESSION_SHELL,
            Self::Agent { .. } => crate::theme::icon::SESSION_AGENT,
        }
    }
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
    /// When this session was created (used for startup spinner animation).
    pub created_at: std::time::Instant,
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

/// Shared queue of terminal Docker lifecycle results produced in the background.
pub type DockerProgressQueue = Arc<Mutex<VecDeque<DockerProgressResult>>>;

/// Shared queue of branch-detail preload results produced in the background.
pub type BranchDetailQueue = Arc<Mutex<VecDeque<BranchDetailLoadResult>>>;

#[cfg(test)]
pub(crate) type BranchDetailDockerSnapshotter =
    Arc<dyn Fn() -> Vec<gwt_docker::ContainerInfo> + Send + Sync>;

/// Tracked branch-detail preload worker state.
pub(crate) struct BranchDetailWorker {
    events: BranchDetailQueue,
    cancel: Arc<AtomicBool>,
    active: Option<JoinHandle<()>>,
    retired: Vec<JoinHandle<()>>,
}

impl BranchDetailWorker {
    pub(crate) fn new(
        events: BranchDetailQueue,
        cancel: Arc<AtomicBool>,
        active: JoinHandle<()>,
    ) -> Self {
        Self {
            events,
            cancel,
            active: Some(active),
            retired: Vec::new(),
        }
    }

    pub(crate) fn events(&self) -> BranchDetailQueue {
        self.events.clone()
    }

    pub(crate) fn replace(
        &mut self,
        events: BranchDetailQueue,
        cancel: Arc<AtomicBool>,
        active: JoinHandle<()>,
    ) {
        self.cancel_active();
        if let Some(handle) = self.active.take() {
            self.retired.push(handle);
        }
        self.reap_finished();
        self.events = events;
        self.cancel = cancel;
        self.active = Some(active);
    }

    pub(crate) fn reap_finished(&mut self) {
        if self
            .active
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            if let Some(handle) = self.active.take() {
                let _ = handle.join();
            }
        }

        let mut index = 0;
        while index < self.retired.len() {
            if self.retired[index].is_finished() {
                let handle = self.retired.swap_remove(index);
                let _ = handle.join();
            } else {
                index += 1;
            }
        }
    }

    fn cancel_active(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Drop for BranchDetailWorker {
    fn drop(&mut self) {
        self.cancel_active();
        if let Some(handle) = self.active.take() {
            let _ = handle.join();
        }
        for handle in self.retired.drain(..) {
            let _ = handle.join();
        }
    }
}

impl std::fmt::Debug for BranchDetailWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BranchDetailWorker")
            .field("retired", &self.retired.len())
            .finish()
    }
}

/// Result sent from the background Docker lifecycle worker back into the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerProgressResult {
    Completed { message: String },
    Failed { message: String, detail: String },
}

/// A terminal cell position within the currently visible viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCell {
    pub row: u16,
    pub col: u16,
}

/// A drag-selection range across visible terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSelection {
    pub anchor: TerminalCell,
    pub focus: TerminalCell,
}

/// Minimal vt100 screen state wrapper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenSnapshot {
    rows: u16,
    cols: u16,
    state: Vec<u8>,
    visible_lines: Vec<String>,
}

impl ScreenSnapshot {
    fn from_screen(rows: u16, cols: u16, screen: &vt100::Screen) -> Self {
        Self {
            rows,
            cols,
            state: screen.state_formatted(),
            visible_lines: screen_visible_lines(screen),
        }
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn state(&self) -> &[u8] {
        &self.state
    }

    fn is_blank(&self) -> bool {
        !slice_contains_non_blank_content(&self.visible_lines)
    }

    fn same_visible_surface(&self, other: &Self) -> bool {
        self.rows == other.rows
            && self.cols == other.cols
            && self.visible_lines == other.visible_lines
    }
}

fn screen_visible_lines(screen: &vt100::Screen) -> Vec<String> {
    let (rows, cols) = screen.size();
    (0..rows)
        .map(|row| screen.contents_between(row, 0, row, cols))
        .collect()
}

fn slice_contains_non_blank_content(lines: &[String]) -> bool {
    lines.iter().any(|line| !line.trim().is_empty())
}

const SNAPSHOT_HISTORY_CAPACITY: usize = 2048;

pub struct VtState {
    parser: vt100::Parser,
    rows: u16,
    cols: u16,
    max_scrollback: usize,
    follow_live: bool,
    selection: Option<TerminalSelection>,
    snapshots: VecDeque<ScreenSnapshot>,
    snapshot_cursor: Option<usize>,
}

impl std::fmt::Debug for VtState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VtState")
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .field("max_scrollback", &self.max_scrollback)
            .field("follow_live", &self.follow_live)
            .finish()
    }
}

impl Clone for VtState {
    fn clone(&self) -> Self {
        let mut parser = vt100::Parser::new(self.rows, self.cols, 10_000);
        let state = self.parser.screen().state_formatted();
        parser.process(&state);
        parser.set_scrollback(self.scrollback());
        Self {
            parser,
            rows: self.rows,
            cols: self.cols,
            max_scrollback: self.max_scrollback,
            follow_live: self.follow_live,
            selection: self.selection,
            snapshots: self.snapshots.clone(),
            snapshot_cursor: self.snapshot_cursor,
        }
    }
}

impl VtState {
    pub fn new(rows: u16, cols: u16) -> Self {
        let mut state = Self {
            parser: vt100::Parser::new(rows, cols, 10_000),
            rows,
            cols,
            max_scrollback: 0,
            follow_live: true,
            selection: None,
            snapshots: VecDeque::new(),
            snapshot_cursor: None,
        };
        state.refresh_scrollback_metrics();
        state
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let current_scrollback = self.scrollback();
        self.rows = rows;
        self.cols = cols;
        self.parser.set_size(rows, cols);
        self.refresh_scrollback_metrics();
        self.parser
            .set_scrollback(current_scrollback.min(self.max_scrollback));
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let previous_scrollback = self.scrollback();
        let previous_max_scrollback = self.max_scrollback;
        self.parser.process(bytes);
        self.refresh_scrollback_metrics();
        if self.max_scrollback > 0 {
            self.snapshot_cursor = None;
        }
        if self.follow_live {
            self.parser.set_scrollback(0);
        } else {
            let added_scrollback = self.max_scrollback.saturating_sub(previous_max_scrollback);
            self.parser.set_scrollback(
                previous_scrollback
                    .saturating_add(added_scrollback)
                    .min(self.max_scrollback),
            );
        }
        self.capture_snapshot();
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    pub fn uses_snapshot_scrollback(&self) -> bool {
        self.max_scrollback == 0 || self.parser.screen().alternate_screen()
    }

    pub fn has_viewport_scrollback(&self) -> bool {
        if self.uses_snapshot_scrollback() {
            self.has_snapshot_scrollback()
        } else {
            self.max_scrollback > 0
        }
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        self.parser.set_scrollback(rows.min(self.max_scrollback));
    }

    pub fn follow_live(&self) -> bool {
        self.follow_live
    }

    pub fn set_follow_live(&mut self, follow_live: bool) {
        self.follow_live = follow_live;
        if follow_live {
            self.parser.set_scrollback(0);
            self.snapshot_cursor = None;
        }
    }

    pub fn has_snapshot_scrollback(&self) -> bool {
        self.uses_snapshot_scrollback() && self.snapshots.len() > 1
    }

    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    pub fn snapshot_position(&self) -> usize {
        if self.snapshots.is_empty() {
            0
        } else {
            self.snapshot_cursor
                .unwrap_or(self.snapshots.len().saturating_sub(1))
        }
    }

    pub fn snapshot_parser(&self) -> Option<vt100::Parser> {
        let snapshot = self.active_snapshot()?;
        let mut parser = vt100::Parser::new(snapshot.rows(), snapshot.cols(), 0);
        parser.process(snapshot.state());
        Some(parser)
    }

    pub fn visible_screen_parser(&self) -> vt100::Parser {
        if let Some(snapshot) = self.active_snapshot() {
            let mut parser = vt100::Parser::new(snapshot.rows(), snapshot.cols(), 0);
            parser.process(snapshot.state());
            return parser;
        }

        let mut parser = vt100::Parser::new(self.rows, self.cols, 10_000);
        let state = self.parser.screen().state_formatted();
        parser.process(&state);
        parser.set_scrollback(self.scrollback());
        parser
    }

    pub fn viewing_history(&self) -> bool {
        self.active_snapshot().is_some()
    }

    pub fn scroll_snapshot_up(&mut self, rows: usize) -> bool {
        if rows == 0 || !self.has_snapshot_scrollback() {
            return false;
        }

        let newest_snapshot = self.snapshots.len().saturating_sub(1);
        let oldest_past_snapshot = self.snapshots.len().saturating_sub(2);
        let base = self.snapshot_cursor.unwrap_or(newest_snapshot);
        self.snapshot_cursor = Some(base.saturating_sub(rows).min(oldest_past_snapshot));
        self.follow_live = false;
        true
    }

    pub fn scroll_snapshot_down(&mut self, rows: usize) -> bool {
        let Some(current) = self.snapshot_cursor else {
            return false;
        };
        if rows == 0 {
            return false;
        }

        let last_snapshot = self.snapshots.len().saturating_sub(1);
        let next = current.saturating_add(rows);
        if next >= last_snapshot {
            self.snapshot_cursor = None;
            self.follow_live = true;
        } else {
            self.snapshot_cursor = Some(next);
            self.follow_live = false;
        }
        true
    }

    pub fn scroll_viewport_lines(&mut self, delta_rows: i16) -> bool {
        if delta_rows == 0 {
            return false;
        }

        if self.uses_snapshot_scrollback() {
            if delta_rows > 0 {
                return self.scroll_snapshot_up(delta_rows as usize);
            }
            return self.scroll_snapshot_down(delta_rows.unsigned_abs() as usize);
        }

        if delta_rows > 0 {
            let next = self
                .scrollback()
                .saturating_add(delta_rows as usize)
                .min(self.max_scrollback());
            self.set_follow_live(false);
            self.set_scrollback(next);
            return true;
        }

        let next = self
            .scrollback()
            .saturating_sub(delta_rows.unsigned_abs() as usize);
        self.set_scrollback(next);
        self.set_follow_live(next == 0);
        true
    }

    pub fn scrollbar_metrics(&self, viewport_height: usize) -> Option<(usize, usize, usize)> {
        if self.uses_snapshot_scrollback() {
            if !self.has_snapshot_scrollback() {
                return None;
            }
            let visible_viewport = viewport_height.max(1);
            return Some((
                self.snapshot_count()
                    .saturating_sub(1)
                    .saturating_add(visible_viewport),
                self.snapshot_position(),
                visible_viewport,
            ));
        }

        if self.max_scrollback() > 0 {
            let content_length = self.max_scrollback().saturating_add(viewport_height);
            let position = self.max_scrollback().saturating_sub(self.scrollback());
            return Some((content_length, position, viewport_height));
        }

        None
    }

    pub fn selection(&self) -> Option<TerminalSelection> {
        self.selection
    }

    pub fn begin_selection(&mut self, cell: TerminalCell) {
        self.selection = Some(TerminalSelection {
            anchor: cell,
            focus: cell,
        });
    }

    pub fn update_selection(&mut self, cell: TerminalCell) {
        if let Some(mut selection) = self.selection {
            selection.focus = cell;
            self.selection = Some(selection);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    fn refresh_scrollback_metrics(&mut self) {
        let current_scrollback = self.parser.screen().scrollback();
        self.parser.set_scrollback(usize::MAX);
        self.max_scrollback = self.parser.screen().scrollback();
        self.parser
            .set_scrollback(current_scrollback.min(self.max_scrollback));
    }

    fn active_snapshot(&self) -> Option<&ScreenSnapshot> {
        if !self.uses_snapshot_scrollback() {
            return None;
        }
        self.snapshot_cursor
            .and_then(|index| self.snapshots.get(index))
    }

    fn capture_snapshot(&mut self) {
        if !self.uses_snapshot_scrollback() {
            return;
        }

        let snapshot = ScreenSnapshot::from_screen(self.rows, self.cols, self.parser.screen());

        if self
            .snapshots
            .back()
            .is_some_and(|existing| existing.same_visible_surface(&snapshot))
        {
            return;
        }

        self.snapshots.push_back(snapshot);
        if self.snapshots.len() > SNAPSHOT_HISTORY_CAPACITY {
            self.snapshots.pop_front();
            if let Some(cursor) = self.snapshot_cursor {
                self.snapshot_cursor = Some(cursor.saturating_sub(1));
            }
        }
        self.prune_leading_blank_snapshots();
    }

    fn prune_leading_blank_snapshots(&mut self) {
        while self.snapshots.len() > 1
            && self.snapshots.front().is_some_and(ScreenSnapshot::is_blank)
        {
            self.snapshots.pop_front();
            if let Some(cursor) = self.snapshot_cursor {
                self.snapshot_cursor = Some(cursor.saturating_sub(1));
            }
        }
    }
}

/// Central application state.
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
    /// Whether the help overlay is visible.
    pub(crate) help_visible: bool,
    /// Error queue (shown as overlays).
    pub(crate) error_queue: VecDeque<Notification>,
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
    /// Background Docker lifecycle completion queue polled from the tick loop.
    pub(crate) docker_progress_events: Option<DockerProgressQueue>,
    /// Tracked branch-detail preload worker and completion queue polled from the tick loop.
    pub(crate) branch_detail_worker: Option<BranchDetailWorker>,
    /// Test-only override for branch-detail docker snapshots.
    #[cfg(test)]
    pub(crate) branch_detail_docker_snapshotter: Option<BranchDetailDockerSnapshotter>,
    /// Service selection overlay state.
    pub(crate) service_select: Option<ServiceSelectState>,
    /// Port conflict resolution overlay state.
    pub(crate) port_select: Option<PortSelectState>,
    /// Confirmation dialog state.
    pub(crate) confirm: ConfirmState,
    /// Pending session conversion awaiting confirmation.
    pub(crate) pending_session_conversion: Option<PendingSessionConversion>,
    /// Launch config built from completed wizard, ready for PTY spawn.
    pub(crate) pending_launch_config: Option<gwt_agent::LaunchConfig>,
    /// Voice input state.
    pub(crate) voice: VoiceInputState,
    /// Runtime voice session used to bridge start/stop/transcribe.
    pub(crate) voice_runtime: VoiceRuntimeState,
    /// Buffered PTY input generated from forwarded key events.
    pub(crate) pending_pty_inputs: VecDeque<PendingPtyInput>,
    /// Live PTY handles keyed by session id.
    pub(crate) pty_handles: HashMap<String, gwt_terminal::PtyHandle>,
    /// Last observed row for Terminal.app style right-drag trackpad fallback.
    pub(crate) terminal_trackpad_scroll_row: Option<u16>,
    /// Sender for PTY output from background reader threads.
    pub(crate) pty_output_tx: std::sync::mpsc::Sender<(String, Vec<u8>)>,
    /// Receiver for PTY output drained in the event loop.
    pub(crate) pty_output_rx: std::sync::mpsc::Receiver<(String, Vec<u8>)>,
    /// Initialization screen state (present when ActiveLayer::Initialization).
    pub(crate) initialization: Option<InitializationState>,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("active_layer", &self.active_layer)
            .field("active_focus", &self.active_focus)
            .field("sessions", &self.sessions.len())
            .field("active_session", &self.active_session)
            .field("pty_handles", &self.pty_handles.len())
            .field(
                "terminal_trackpad_scroll_row",
                &self.terminal_trackpad_scroll_row,
            )
            .field("repo_path", &self.repo_path)
            .finish()
    }
}

impl Model {
    /// Create a new Model with sensible defaults.
    pub fn new(repo_path: PathBuf) -> Self {
        let default_session = SessionTab {
            id: "shell-0".to_string(),
            name: "Shell".to_string(),
            tab_type: SessionTabType::Shell,
            vt: VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        };
        let (notification_bus, notification_receiver) = NotificationBus::new();
        let (pty_output_tx, pty_output_rx) = std::sync::mpsc::channel();
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
            help_visible: false,
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
            docker_progress_events: None,
            branch_detail_worker: None,
            #[cfg(test)]
            branch_detail_docker_snapshotter: None,
            service_select: None,
            port_select: None,
            confirm: ConfirmState::default(),
            pending_session_conversion: None,
            pending_launch_config: None,
            voice: VoiceInputState::default(),
            voice_runtime: VoiceRuntimeState::default(),
            pending_pty_inputs: VecDeque::new(),
            pty_handles: HashMap::new(),
            terminal_trackpad_scroll_row: None,
            pty_output_tx,
            pty_output_rx,
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
        // Kill all live PTY processes before discarding handles.
        for (_, pty) in self.pty_handles.drain() {
            let _ = pty.kill();
        }
        let terminal_size = self.terminal_size;
        *self = Self::new(repo_path);
        self.terminal_size = terminal_size;
    }

    /// Number of sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    #[cfg(test)]
    pub(crate) fn set_branch_detail_docker_snapshotter<F>(&mut self, snapshotter: F)
    where
        F: Fn() -> Vec<gwt_docker::ContainerInfo> + Send + Sync + 'static,
    {
        self.branch_detail_docker_snapshotter = Some(Arc::new(snapshotter));
    }

    /// Get the active session, if any.
    pub fn active_session_tab(&self) -> Option<&SessionTab> {
        self.sessions.get(self.active_session)
    }

    /// Get the active session mutably, if any.
    pub fn active_session_tab_mut(&mut self) -> Option<&mut SessionTab> {
        self.sessions.get_mut(self.active_session)
    }

    /// Find a session by its stable id.
    pub fn session_tab_mut(&mut self, session_id: &str) -> Option<&mut SessionTab> {
        self.sessions
            .iter_mut()
            .find(|session| session.id == session_id)
    }

    /// Buffered PTY input awaiting delivery to sessions.
    pub fn pending_pty_inputs(&self) -> &VecDeque<PendingPtyInput> {
        &self.pending_pty_inputs
    }

    /// Current terminal size `(cols, rows)`.
    pub fn terminal_size(&self) -> (u16, u16) {
        self.terminal_size
    }

    /// Drain PTY output from background reader threads.
    ///
    /// Returns an iterator of `(session_id, data)` chunks ready for dispatch.
    pub fn drain_pty_output(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        while let Ok(item) = self.pty_output_rx.try_recv() {
            out.push(item);
        }
        out
    }

    /// Kill and remove all live PTY handles.
    pub fn kill_all_pty(&mut self) {
        for (_, pty) in self.pty_handles.drain() {
            let _ = pty.kill();
        }
    }

    /// Cloneable handle for sending notifications into the TUI.
    #[allow(dead_code)]
    pub(crate) fn notification_bus_handle(&self) -> NotificationBus {
        self._notification_bus.clone()
    }

    /// Repository root currently driving the workspace shell.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
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
        let state = SessionState {
            display_mode: match self.session_layout {
                SessionLayout::Tab => "tab".to_string(),
                SessionLayout::Grid => "grid".to_string(),
            },
            panel_visible: self.active_layer == ActiveLayer::Management,
            active_management_tab: self.management_tab.label().to_string(),
            session_count: self.sessions.len(),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = toml::to_string_pretty(&state).map_err(|e| e.to_string())?;
        std::fs::write(path, content).map_err(|e| e.to_string())
    }

    /// Load session state from a TOML file.
    pub fn load_session_state(path: &Path) -> Result<SessionState, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| e.to_string())
    }

    /// Stable state-file location for a repository root.
    pub fn session_state_path(repo_path: &Path) -> PathBuf {
        let encoded = URL_SAFE_NO_PAD.encode(repo_path.to_string_lossy().as_bytes());
        gwt_core::paths::gwt_sessions_dir().join(format!("{encoded}.toml"))
    }

    /// Restore persisted shell state from disk, returning a warning when fallback was needed.
    pub fn restore_session_state_from_path(&mut self, path: &Path) -> Option<String> {
        if !path.exists() {
            return None;
        }

        match Self::load_session_state(path) {
            Ok(state) => self.apply_session_state(state),
            Err(err) => Some(format!("failed to restore session state: {err}")),
        }
    }

    fn apply_session_state(&mut self, state: SessionState) -> Option<String> {
        let mut warnings = Vec::new();

        self.session_layout = match state.display_mode.as_str() {
            "tab" => SessionLayout::Tab,
            "grid" => SessionLayout::Grid,
            other => {
                warnings.push(format!("unknown display_mode `{other}`"));
                SessionLayout::Tab
            }
        };
        self.active_layer = if state.panel_visible {
            ActiveLayer::Management
        } else {
            ActiveLayer::Main
        };
        self.management_tab = if state.active_management_tab == "Specs" {
            ManagementTab::Branches
        } else {
            match ManagementTab::ALL
                .iter()
                .copied()
                .find(|tab| tab.label() == state.active_management_tab)
            {
                Some(tab) => tab,
                None => {
                    warnings.push(format!(
                        "unknown active_management_tab `{}`",
                        state.active_management_tab
                    ));
                    ManagementTab::Branches
                }
            }
        };

        if warnings.is_empty() {
            None
        } else {
            Some(warnings.join("; "))
        }
    }
}

/// Persisted session layout state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default = "default_display_mode")]
    pub display_mode: String,
    #[serde(default = "default_panel_visible", alias = "management_visible")]
    pub panel_visible: bool,
    #[serde(default = "default_active_management_tab")]
    pub active_management_tab: String,
    #[serde(default = "default_session_count")]
    pub session_count: usize,
}

fn default_display_mode() -> String {
    "tab".to_string()
}

fn default_panel_visible() -> bool {
    false
}

fn default_active_management_tab() -> String {
    "Branches".to_string()
}

fn default_session_count() -> usize {
    1
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            display_mode: default_display_mode(),
            panel_visible: default_panel_visible(),
            active_management_tab: default_active_management_tab(),
            session_count: default_session_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn model_new_defaults() {
        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        assert_eq!(model.active_layer, ActiveLayer::Management);
        assert_eq!(model.session_count(), 1);
        assert_eq!(model.active_session, 0);
        assert_eq!(model.session_layout, SessionLayout::Tab);
        assert_eq!(model.management_tab, ManagementTab::Branches);
        assert!(!model.help_visible);
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
        assert_eq!(
            ManagementTab::ALL
                .iter()
                .map(|tab| tab.label())
                .collect::<Vec<_>>(),
            vec![
                "Branches", "Issues", "PRs", "Profiles", "Git View", "Versions", "Settings",
                "Logs",
            ]
        );
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

    fn full_screen_frame(lines: &[&str]) -> Vec<u8> {
        let mut sequence = String::from("\u{1b}[2J\u{1b}[H");
        for (index, line) in lines.iter().enumerate() {
            sequence.push_str(&format!("\u{1b}[{};1H{}", index + 1, line));
        }
        sequence.into_bytes()
    }

    #[test]
    fn capture_snapshot_skips_identical_consecutive_frames() {
        let mut vt = VtState::new(5, 20);
        let frame = full_screen_frame(&["line-1", "line-2", "line-3", "line-4", "line-5"]);

        vt.process(&frame);
        vt.process(&frame);

        assert_eq!(vt.max_scrollback(), 0);
        assert_eq!(vt.snapshot_count(), 1);
        assert!(!vt.has_snapshot_scrollback());
    }

    #[test]
    fn capture_snapshot_ignores_style_only_redraw_frames() {
        let mut vt = VtState::new(5, 20);
        vt.process(b"\x1b[?1049h\x1b[2J\x1b[Hframe");
        vt.process(b"\x1b[7m\x1b[1;1Hframe\x1b[0m");
        vt.process(b"\x1b[4m\x1b[1;1Hframe\x1b[0m");

        assert_eq!(vt.max_scrollback(), 0);
        assert_eq!(
            vt.snapshot_count(),
            1,
            "style-only redraws should not consume snapshot history"
        );
        assert!(!vt.has_snapshot_scrollback());
    }

    #[test]
    fn capture_snapshot_keeps_distinct_full_screen_redraw_frames() {
        let mut vt = VtState::new(5, 20);
        vt.process(&full_screen_frame(&[
            "line-1", "line-2", "line-3", "line-4", "line-5",
        ]));
        vt.process(&full_screen_frame(&[
            "line-2", "line-3", "line-4", "line-5", "line-6",
        ]));

        assert_eq!(vt.max_scrollback(), 0);
        assert_eq!(vt.snapshot_count(), 2);
        assert!(vt.has_snapshot_scrollback());

        assert!(vt.scroll_snapshot_up(1));
        let snapshot = vt
            .snapshot_parser()
            .expect("snapshot parser should exist when viewing history");
        let contents = snapshot.screen().contents();
        assert!(contents.contains("line-1"));
        assert!(!contents.contains("line-6"));

        assert!(vt.scroll_snapshot_down(1));
        assert!(vt.follow_live());
        assert!(vt.snapshot_parser().is_none());
    }

    #[test]
    fn alternate_screen_uses_snapshot_history_even_with_existing_row_scrollback() {
        let mut vt = VtState::new(5, 20);
        for index in 0..12 {
            vt.process(format!("seed-{index}\r\n").as_bytes());
        }
        assert!(vt.max_scrollback() > 0);
        assert!(!vt.screen().alternate_screen());
        assert!(!vt.uses_snapshot_scrollback());

        vt.process(b"\x1b[?1049h\x1b[2J\x1b[Hframe-1");
        vt.process(b"\x1b[2J\x1b[Hframe-2");

        assert!(vt.screen().alternate_screen());
        assert!(vt.uses_snapshot_scrollback());
        assert!(vt.has_snapshot_scrollback());

        assert!(vt.scroll_snapshot_up(1));
        let snapshot = vt
            .snapshot_parser()
            .expect("snapshot parser should exist while browsing alternate-screen history");
        let contents = snapshot.screen().contents();
        assert!(contents.contains("frame-1"));
        assert!(!contents.contains("frame-2"));
    }

    #[test]
    fn prune_leading_blank_snapshots_discards_only_blank_prefix() {
        let mut vt = VtState::new(5, 20);
        vt.snapshots = VecDeque::from(vec![
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: Vec::new(),
                visible_lines: vec!["".to_string(); 5],
            },
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: Vec::new(),
                visible_lines: vec![
                    "line-1".to_string(),
                    "line-2".to_string(),
                    "line-3".to_string(),
                    "line-4".to_string(),
                    "line-5".to_string(),
                ],
            },
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: Vec::new(),
                visible_lines: vec![
                    "line-2".to_string(),
                    "line-3".to_string(),
                    "line-4".to_string(),
                    "line-5".to_string(),
                    "line-6".to_string(),
                ],
            },
        ]);
        vt.snapshot_cursor = Some(0);

        vt.prune_leading_blank_snapshots();

        assert_eq!(vt.snapshots.len(), 2);
        assert!(
            !vt.snapshots.front().expect("front snapshot").is_blank(),
            "the oldest remaining snapshot should carry visible content"
        );
        assert_eq!(
            vt.snapshot_cursor,
            Some(0),
            "cursor should stay clamped to the oldest remaining meaningful frame"
        );
    }

    #[test]
    fn scroll_snapshot_up_from_live_moves_exactly_one_snapshot() {
        let mut vt = VtState::new(5, 20);
        vt.max_scrollback = 0;
        vt.follow_live = true;
        vt.snapshots = VecDeque::from(vec![
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: vec![1],
                visible_lines: vec!["frame-1".to_string(); 5],
            },
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: vec![2],
                visible_lines: vec!["frame-2".to_string(); 5],
            },
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: vec![3],
                visible_lines: vec!["frame-3".to_string(); 5],
            },
        ]);

        let changed = vt.scroll_snapshot_up(1);

        assert!(changed);
        assert_eq!(
            vt.snapshot_cursor,
            Some(1),
            "first upward step from live should land on the immediately previous snapshot"
        );
        assert!(!vt.follow_live);
    }

    // ---- SessionState tests ----

    #[test]
    fn session_state_default() {
        let state = SessionState::default();
        assert_eq!(state.display_mode, "tab");
        assert!(!state.panel_visible);
        assert_eq!(state.active_management_tab, "Branches");
        assert_eq!(state.session_count, 1);
    }

    #[test]
    fn save_and_load_session_state_roundtrip_preserves_layout_visibility_and_tab() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        let model = Model::new(PathBuf::from("/tmp/repo"));
        model.save_session_state(&path).unwrap();

        let loaded = Model::load_session_state(&path).expect("load state");
        assert_eq!(loaded.display_mode, "tab");
        assert!(loaded.panel_visible);
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

        let loaded = Model::load_session_state(&path).expect("load state");
        assert_eq!(loaded.display_mode, "grid");
        assert!(loaded.panel_visible);
        assert_eq!(loaded.active_management_tab, "Settings");
    }

    #[test]
    fn load_session_state_missing_file_returns_error() {
        let result = Model::load_session_state(Path::new("/nonexistent/path/session.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn branch_detail_worker_drop_waits_for_worker_exit() {
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let completed_flag = completed.clone();
        let cancel_flag = cancel.clone();
        let handle = std::thread::spawn(move || {
            while !cancel_flag.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(20));
            completed_flag.store(true, Ordering::SeqCst);
        });

        {
            let _worker = BranchDetailWorker::new(events, cancel, handle);
        }

        assert!(
            completed.load(Ordering::SeqCst),
            "dropping the worker should wait for the cancelled thread to exit"
        );
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
            created_at: std::time::Instant::now(),
        });
        model.sessions.push(SessionTab {
            id: "shell-2".to_string(),
            name: "Shell 3".to_string(),
            tab_type: SessionTabType::Shell,
            vt: VtState::new(24, 80),
            created_at: std::time::Instant::now(),
        });
        model.save_session_state(&path).unwrap();

        let loaded = Model::load_session_state(&path).expect("load state");
        assert_eq!(loaded.session_count, 3);
    }

    #[test]
    fn save_session_state_creates_missing_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.toml");

        let model = Model::new(PathBuf::from("/tmp/repo"));
        model.save_session_state(&path).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn restore_session_state_from_corrupted_file_returns_warning_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "display_mode = [").unwrap();

        let mut model = Model::new(PathBuf::from("/tmp/repo"));
        model.session_layout = SessionLayout::Grid;
        model.management_tab = ManagementTab::Settings;

        let warning = model.restore_session_state_from_path(&path);

        assert!(warning.is_some());
        assert_eq!(model.session_layout, SessionLayout::Grid);
        assert_eq!(model.management_tab, ManagementTab::Settings);
    }

    #[test]
    fn restore_session_state_from_path_applies_saved_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        let mut original = Model::new(PathBuf::from("/tmp/repo"));
        original.session_layout = SessionLayout::Grid;
        original.active_layer = ActiveLayer::Main;
        original.management_tab = ManagementTab::Logs;
        original.save_session_state(&path).unwrap();

        let mut restored = Model::new(PathBuf::from("/tmp/repo"));
        let warning = restored.restore_session_state_from_path(&path);

        assert!(warning.is_none());
        assert_eq!(restored.session_layout, SessionLayout::Grid);
        assert_eq!(restored.active_layer, ActiveLayer::Main);
        assert_eq!(restored.management_tab, ManagementTab::Logs);
    }

    #[test]
    fn restore_session_state_maps_legacy_specs_tab_to_branches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(
            &path,
            r#"
display_mode = "tab"
panel_visible = true
active_management_tab = "Specs"
session_count = 1
"#,
        )
        .unwrap();

        let mut restored = Model::new(PathBuf::from("/tmp/repo"));
        let warning = restored.restore_session_state_from_path(&path);

        assert!(warning.is_none());
        assert_eq!(restored.management_tab, ManagementTab::Branches);
    }
}
