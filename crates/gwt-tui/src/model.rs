//! Model — central application state for the Elm Architecture.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
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
    /// SPEC-12 Phase 9: dedicated Specs tab backed by `~/.gwt/cache/issues/`.
    /// Displayed as a top-level peer of Branches/Issues/PRs now that SPECs
    /// live as GitHub Issues rather than worktree-local files.
    Specs,
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
        ManagementTab::Issues,
        ManagementTab::PrDashboard,
        ManagementTab::Specs,
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
            Self::Specs => "Specs",
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

/// Per-branch event emitted by the Branch Cleanup runner background job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupEvent {
    /// Cleanup of `branch` started.
    Started { branch: String },
    /// Cleanup of `branch` finished.
    Finished {
        branch: String,
        success: bool,
        message: Option<String>,
    },
    /// All branches in the run finished.
    Completed,
}

/// Shared queue of cleanup runner events drained from the tick loop.
pub type CleanupEventQueue = Arc<Mutex<VecDeque<CleanupEvent>>>;

/// Background event delivering a finished merge-state computation for one
/// branch back into the Branches model (FR-018d).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeStateEvent {
    pub branch: String,
    pub state: crate::screens::branches::MergeState,
}

/// Shared queue of merge-state events drained from the tick loop.
pub type MergeStateQueue = Arc<Mutex<VecDeque<MergeStateEvent>>>;

/// Tracked merge-state worker handle: a queue plus an explicit finished
/// flag the worker sets when its loop completes. Without the flag the tick
/// loop's drain helper would have to guess from queue emptiness, racing
/// against single-event pushes and tearing the queue down before the
/// remaining branches were ever delivered (FR-018d).
#[derive(Debug, Clone)]
pub struct MergeStateChannel {
    pub queue: MergeStateQueue,
    pub finished: Arc<std::sync::atomic::AtomicBool>,
}

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
    formatted_rows: Vec<Vec<u8>>,
}

impl ScreenSnapshot {
    fn from_screen(rows: u16, cols: u16, screen: &vt100::Screen) -> Self {
        Self {
            rows,
            cols,
            state: screen.state_formatted(),
            visible_lines: screen_visible_lines(screen),
            formatted_rows: screen_formatted_rows(screen),
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
        .map(|row| normalize_visible_line(&screen.contents_between(row, 0, row, cols)))
        .collect()
}

fn screen_formatted_rows(screen: &vt100::Screen) -> Vec<Vec<u8>> {
    let (_, cols) = screen.size();
    screen.rows_formatted(0, cols).collect()
}

fn normalize_visible_line(line: &str) -> String {
    line.trim_end_matches(' ').to_string()
}

fn slice_contains_non_blank_content(lines: &[String]) -> bool {
    lines.iter().any(|line| !line.trim().is_empty())
}

const SNAPSHOT_HISTORY_CAPACITY: usize = 2048;
const ROW_SCROLLBACK_CAPACITY: usize = 10_000;
const AGENT_ROW_SCROLLBACK_CAPACITY: usize = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbackStrategy {
    Standard,
    AgentMemoryBacked,
}

#[derive(Debug, Clone)]
struct SnapshotCaptureOutcome {
    attempted: bool,
    appended: bool,
    deduped: bool,
    pruned_blank_prefix: usize,
    snapshot_count_after: usize,
    synthetic_rows_appended: usize,
    surface_digest: u64,
    top_preview: String,
    bottom_preview: String,
}

impl SnapshotCaptureOutcome {
    fn skipped(snapshot_count: usize) -> Self {
        Self {
            attempted: false,
            appended: false,
            deduped: false,
            pruned_blank_prefix: 0,
            snapshot_count_after: snapshot_count,
            synthetic_rows_appended: 0,
            surface_digest: 0,
            top_preview: String::new(),
            bottom_preview: String::new(),
        }
    }
}

fn is_alt_screen_toggle_sequence(sequence: &[u8]) -> bool {
    matches!(
        sequence,
        b"\x1b[?1049h"
            | b"\x1b[?1049l"
            | b"\x1b[?1047h"
            | b"\x1b[?1047l"
            | b"\x1b[?47h"
            | b"\x1b[?47l"
    )
}

fn filter_scrollback_bytes_with_pending(pending: &mut Vec<u8>, bytes: &[u8]) -> Vec<u8> {
    let mut input = std::mem::take(pending);
    input.extend_from_slice(bytes);

    let mut filtered = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] != 0x1b {
            filtered.push(input[index]);
            index += 1;
            continue;
        }

        if index + 1 >= input.len() {
            pending.extend_from_slice(&input[index..]);
            break;
        }

        if input[index + 1] != b'[' {
            filtered.push(input[index]);
            index += 1;
            continue;
        }

        let mut end = index + 2;
        while end < input.len() && !(0x40..=0x7e).contains(&input[end]) {
            end += 1;
        }
        if end >= input.len() {
            pending.extend_from_slice(&input[index..]);
            break;
        }

        let sequence = &input[index..=end];
        if !is_alt_screen_toggle_sequence(sequence) {
            filtered.extend_from_slice(sequence);
        }
        index = end + 1;
    }

    filtered
}

fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }

    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn split_agent_snapshot_segments(bytes: &[u8]) -> Vec<&[u8]> {
    const CLEAR_HOME: &[u8] = b"\x1b[2J\x1b[H";

    if bytes.is_empty() {
        return Vec::new();
    }

    let mut starts = vec![0usize];
    let mut search_index = 0usize;
    while search_index + CLEAR_HOME.len() <= bytes.len() {
        let Some(relative) = bytes[search_index..]
            .windows(CLEAR_HOME.len())
            .position(|window| window == CLEAR_HOME)
        else {
            break;
        };
        let absolute = search_index + relative;
        if absolute != 0 {
            starts.push(absolute);
        }
        search_index = absolute.saturating_add(CLEAR_HOME.len());
    }

    starts.sort_unstable();
    starts.dedup();

    let mut segments = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(bytes.len());
        if *start < end {
            segments.push(&bytes[*start..end]);
        }
    }
    segments
}

fn segment_contains_clear_home(bytes: &[u8]) -> bool {
    bytes
        .windows(b"\x1b[2J\x1b[H".len())
        .any(|window| window == b"\x1b[2J\x1b[H")
}

fn detect_vertical_redraw_shift(
    previous: &ScreenSnapshot,
    current: &ScreenSnapshot,
) -> Option<(usize, usize)> {
    let row_count = previous
        .visible_lines
        .len()
        .min(current.visible_lines.len());
    if row_count < 2 || previous.visible_lines == current.visible_lines {
        return None;
    }

    let mut best_match: Option<(usize, usize, usize)> = None;
    for shift in 1..row_count {
        for window_start in 0..row_count.saturating_sub(shift + 1) {
            for window_end in (window_start + shift + 2)..=row_count {
                let overlap = window_end.saturating_sub(window_start + shift);
                let previous_overlap = &previous.visible_lines[window_start + shift..window_end];
                let current_overlap = &current.visible_lines[window_start..window_end - shift];
                let overlap_contains_non_blank =
                    previous_overlap.iter().any(|line| !line.trim().is_empty());
                let introduced_non_blank = current.visible_lines[window_end - shift..window_end]
                    .iter()
                    .any(|line| !line.trim().is_empty());

                if previous_overlap != current_overlap
                    || !overlap_contains_non_blank
                    || !introduced_non_blank
                {
                    continue;
                }

                let candidate = (window_start, shift, overlap);
                if best_match.is_none_or(|best| candidate.2 > best.2) {
                    best_match = Some(candidate);
                }
            }
        }
    }

    if let Some((window_start, shift, _)) = best_match {
        return Some((window_start, shift));
    }

    detect_sparse_vertical_redraw_shift(previous, current)
}

fn detect_sparse_vertical_redraw_shift(
    previous: &ScreenSnapshot,
    current: &ScreenSnapshot,
) -> Option<(usize, usize)> {
    let row_count = previous
        .visible_lines
        .len()
        .min(current.visible_lines.len());
    if row_count < 2 {
        return None;
    }

    let minimum_matches = if row_count <= 4 { 1 } else { 2 };
    let mut best_match: Option<(usize, usize, usize, usize)> = None;
    for shift in 1..row_count {
        let mut matches = Vec::new();
        for current_index in 0..row_count.saturating_sub(shift) {
            let previous_line = &previous.visible_lines[current_index + shift];
            let current_line = &current.visible_lines[current_index];
            if previous_line == current_line && !current_line.trim().is_empty() {
                matches.push(current_index);
            }
        }

        if matches.len() < minimum_matches {
            continue;
        }

        let window_start = matches[0];
        let shifted_off_contains_non_blank = previous.visible_lines
            [window_start..window_start.saturating_add(shift)]
            .iter()
            .any(|line| !line.trim().is_empty());
        if !shifted_off_contains_non_blank {
            continue;
        }

        let span = matches.last().copied().unwrap_or(window_start) - window_start + 1;
        let candidate = (matches.len(), window_start, span, shift);
        if best_match.is_none_or(|best| {
            candidate.0 > best.0
                || (candidate.0 == best.0
                    && (candidate.1 < best.1
                        || (candidate.1 == best.1
                            && (candidate.2 > best.2
                                || (candidate.2 == best.2 && candidate.3 < best.3)))))
        }) {
            best_match = Some(candidate);
        }
    }

    best_match.map(|(_, window_start, _, shift)| (window_start, shift))
}

fn preview_visible_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let preview: String = trimmed.chars().take(48).collect();
    preview.replace(' ', "_")
}

fn visible_surface_digest(lines: &[String]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut hasher);
    hasher.finish()
}

fn first_non_blank_preview(lines: &[String]) -> String {
    lines
        .iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| preview_visible_line(line))
        .unwrap_or_default()
}

fn last_non_blank_preview(lines: &[String]) -> String {
    lines
        .iter()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| preview_visible_line(line))
        .unwrap_or_default()
}

pub struct VtState {
    parser: vt100::Parser,
    scrollback_parser: vt100::Parser,
    rows: u16,
    cols: u16,
    max_scrollback: usize,
    agent_scrollback: usize,
    follow_live: bool,
    selection: Option<TerminalSelection>,
    agent_row_history: VecDeque<Vec<u8>>,
    snapshots: VecDeque<ScreenSnapshot>,
    snapshot_cursor: Option<usize>,
    scrollback_strategy: ScrollbackStrategy,
    scrollback_filter_pending: Vec<u8>,
    mouse_mode_pending: Vec<u8>,
    mouse_tracking_enabled: bool,
    sgr_mouse_enabled: bool,
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
        let mut parser = vt100::Parser::new(self.rows, self.cols, ROW_SCROLLBACK_CAPACITY);
        let state = self.parser.screen().state_formatted();
        parser.process(&state);
        parser.set_scrollback(0);
        let mut scrollback_parser =
            vt100::Parser::new(self.rows, self.cols, self.row_scrollback_capacity());
        let scrollback_state = self.scrollback_parser.screen().state_formatted();
        scrollback_parser.process(&scrollback_state);
        scrollback_parser.set_scrollback(self.scrollback());
        Self {
            parser,
            scrollback_parser,
            rows: self.rows,
            cols: self.cols,
            max_scrollback: self.max_scrollback,
            agent_scrollback: self.agent_scrollback,
            follow_live: self.follow_live,
            selection: self.selection,
            agent_row_history: self.agent_row_history.clone(),
            snapshots: self.snapshots.clone(),
            snapshot_cursor: self.snapshot_cursor,
            scrollback_strategy: self.scrollback_strategy,
            scrollback_filter_pending: self.scrollback_filter_pending.clone(),
            mouse_mode_pending: self.mouse_mode_pending.clone(),
            mouse_tracking_enabled: self.mouse_tracking_enabled,
            sgr_mouse_enabled: self.sgr_mouse_enabled,
        }
    }
}

impl VtState {
    pub fn new(rows: u16, cols: u16) -> Self {
        let mut state = Self {
            parser: vt100::Parser::new(rows, cols, ROW_SCROLLBACK_CAPACITY),
            scrollback_parser: vt100::Parser::new(rows, cols, ROW_SCROLLBACK_CAPACITY),
            rows,
            cols,
            max_scrollback: 0,
            agent_scrollback: 0,
            follow_live: true,
            selection: None,
            agent_row_history: VecDeque::new(),
            snapshots: VecDeque::new(),
            snapshot_cursor: None,
            scrollback_strategy: ScrollbackStrategy::Standard,
            scrollback_filter_pending: Vec::new(),
            mouse_mode_pending: Vec::new(),
            mouse_tracking_enabled: false,
            sgr_mouse_enabled: false,
        };
        state.refresh_scrollback_metrics();
        state
    }

    pub fn set_scrollback_strategy(&mut self, strategy: ScrollbackStrategy) {
        if self.scrollback_strategy == strategy {
            return;
        }

        self.scrollback_strategy = strategy;
        self.scrollback_filter_pending.clear();
        self.mouse_mode_pending.clear();
        self.agent_row_history.clear();
        self.agent_scrollback = 0;
        self.scrollback_parser =
            vt100::Parser::new(self.rows, self.cols, self.row_scrollback_capacity());
        let live_state = self.parser.screen().state_formatted();
        self.scrollback_parser.process(&live_state);
        if matches!(strategy, ScrollbackStrategy::AgentMemoryBacked) {
            self.snapshots.clear();
            self.snapshot_cursor = None;
        }
        self.refresh_scrollback_metrics();
        self.set_scrollback(0);
        self.capture_snapshot();
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
        self.scrollback_parser.set_size(rows, cols);
        self.refresh_scrollback_metrics();
        self.set_scrollback(current_scrollback.min(self.max_scrollback));
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let previous_scrollback = self.scrollback();
        let previous_max_scrollback = self.max_scrollback;
        let previous_snapshot_count = self.snapshots.len();
        let mut synthetic_rows_appended = 0usize;
        let segments = if matches!(
            self.scrollback_strategy,
            ScrollbackStrategy::AgentMemoryBacked
        ) {
            split_agent_snapshot_segments(bytes)
        } else {
            vec![bytes]
        };

        for (index, segment) in segments.iter().enumerate() {
            self.update_mouse_reporting_modes(segment);
            self.parser.process(segment);
            let current_snapshot =
                ScreenSnapshot::from_screen(self.rows, self.cols, self.parser.screen());
            let synthetic_rows = self.synthetic_scrollback_rows(segment, &current_snapshot);
            synthetic_rows_appended = synthetic_rows_appended.saturating_add(synthetic_rows.len());
            self.process_scrollback_bytes(segment, &synthetic_rows, &current_snapshot);
            if index + 1 < segments.len() {
                self.capture_snapshot();
            }
        }
        self.refresh_scrollback_metrics();
        if self.max_scrollback > 0 && self.follow_live {
            self.snapshot_cursor = None;
        }
        if self.follow_live {
            self.set_scrollback(0);
        } else {
            let added_scrollback = self.max_scrollback.saturating_sub(previous_max_scrollback);
            self.set_scrollback(
                previous_scrollback
                    .saturating_add(added_scrollback)
                    .min(self.max_scrollback),
            );
        }
        let mut snapshot_outcome = self.capture_snapshot();
        snapshot_outcome.synthetic_rows_appended = synthetic_rows_appended;
        self.log_process_debug(
            bytes,
            previous_max_scrollback,
            previous_snapshot_count,
            &snapshot_outcome,
        );
    }

    fn process_scrollback_bytes(
        &mut self,
        bytes: &[u8],
        synthetic_rows: &[Vec<u8>],
        current_snapshot: &ScreenSnapshot,
    ) {
        if matches!(
            self.scrollback_strategy,
            ScrollbackStrategy::AgentMemoryBacked
        ) {
            self.append_agent_row_history(synthetic_rows);
            if segment_contains_clear_home(bytes) {
                self.scrollback_parser.process(b"\x1b[2J\x1b[H");
                self.scrollback_parser.process(current_snapshot.state());
                return;
            }
            let filtered =
                filter_scrollback_bytes_with_pending(&mut self.scrollback_filter_pending, bytes);
            if !filtered.is_empty() {
                self.scrollback_parser.process(&filtered);
            }
        } else {
            self.scrollback_parser.process(bytes);
        }
    }

    fn synthetic_scrollback_rows(
        &self,
        segment: &[u8],
        current_snapshot: &ScreenSnapshot,
    ) -> Vec<Vec<u8>> {
        if !matches!(
            self.scrollback_strategy,
            ScrollbackStrategy::AgentMemoryBacked
        ) || segment.is_empty()
        {
            return Vec::new();
        }

        let Some(previous_snapshot) = self.snapshots.back() else {
            return Vec::new();
        };

        let Some((window_start, shift)) =
            detect_vertical_redraw_shift(previous_snapshot, current_snapshot)
        else {
            return Vec::new();
        };

        previous_snapshot
            .formatted_rows
            .iter()
            .skip(window_start)
            .take(shift)
            .cloned()
            .collect()
    }

    fn update_mouse_reporting_modes(&mut self, bytes: &[u8]) {
        let mut input = std::mem::take(&mut self.mouse_mode_pending);
        input.extend_from_slice(bytes);

        let mut index = 0;
        while index < input.len() {
            if input[index] != 0x1b {
                index += 1;
                continue;
            }

            if index + 1 >= input.len() {
                self.mouse_mode_pending.extend_from_slice(&input[index..]);
                break;
            }

            if input[index + 1] != b'[' {
                index += 1;
                continue;
            }

            let mut end = index + 2;
            while end < input.len() && !(0x40..=0x7e).contains(&input[end]) {
                end += 1;
            }
            if end >= input.len() {
                self.mouse_mode_pending.extend_from_slice(&input[index..]);
                break;
            }

            match &input[index..=end] {
                b"\x1b[?1000h" | b"\x1b[?1002h" | b"\x1b[?1003h" => {
                    self.mouse_tracking_enabled = true;
                }
                b"\x1b[?1000l" | b"\x1b[?1002l" | b"\x1b[?1003l" => {
                    self.mouse_tracking_enabled = false;
                }
                b"\x1b[?1006h" => {
                    self.sgr_mouse_enabled = true;
                }
                b"\x1b[?1006l" => {
                    self.sgr_mouse_enabled = false;
                }
                _ => {}
            }
            index = end + 1;
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn scrollback(&self) -> usize {
        if self.uses_agent_row_history() {
            return self.agent_scrollback;
        }
        self.scrollback_parser.screen().scrollback()
    }

    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }

    pub fn uses_snapshot_scrollback(&self) -> bool {
        if self.snapshot_history_locked() {
            return true;
        }

        match self.scrollback_strategy {
            ScrollbackStrategy::AgentMemoryBacked => self.max_scrollback == 0,
            ScrollbackStrategy::Standard => {
                self.max_scrollback == 0 || self.parser.screen().alternate_screen()
            }
        }
    }

    pub fn has_viewport_scrollback(&self) -> bool {
        if self.uses_snapshot_scrollback() {
            self.has_snapshot_scrollback()
        } else {
            self.max_scrollback > 0
        }
    }

    pub fn set_scrollback(&mut self, rows: usize) {
        if self.uses_agent_row_history() {
            self.agent_scrollback = rows.min(self.max_scrollback);
            return;
        }
        self.scrollback_parser
            .set_scrollback(rows.min(self.max_scrollback));
    }

    pub fn follow_live(&self) -> bool {
        self.follow_live
    }

    pub fn accepts_mouse_scroll_input(&self) -> bool {
        self.mouse_tracking_enabled && self.sgr_mouse_enabled
    }

    pub fn set_follow_live(&mut self, follow_live: bool) {
        self.follow_live = follow_live;
        if follow_live {
            self.set_scrollback(0);
            self.snapshot_cursor = None;
        }
    }

    fn row_scrollback_capacity(&self) -> usize {
        match self.scrollback_strategy {
            ScrollbackStrategy::Standard => ROW_SCROLLBACK_CAPACITY,
            ScrollbackStrategy::AgentMemoryBacked => AGENT_ROW_SCROLLBACK_CAPACITY,
        }
    }

    pub fn has_snapshot_scrollback(&self) -> bool {
        self.snapshots.len() > 1 && self.uses_snapshot_scrollback()
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

        if !self.follow_live && self.max_scrollback() > 0 {
            if self.uses_agent_row_history() {
                return self.agent_row_history_parser();
            }
            let mut parser =
                vt100::Parser::new(self.rows, self.cols, self.row_scrollback_capacity());
            let state = self.scrollback_parser.screen().state_formatted();
            parser.process(&state);
            parser.set_scrollback(self.scrollback());
            return parser;
        }

        let mut parser = vt100::Parser::new(self.rows, self.cols, self.row_scrollback_capacity());
        let state = self.parser.screen().state_formatted();
        parser.process(&state);
        parser
    }

    pub fn viewing_history(&self) -> bool {
        self.active_snapshot().is_some() || (!self.follow_live && self.max_scrollback() > 0)
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

        if delta_rows > 0 {
            return self.scroll_viewport_up(delta_rows as usize);
        }

        self.scroll_viewport_down(delta_rows.unsigned_abs() as usize)
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
        let current_scrollback = self.scrollback_parser.screen().scrollback();
        self.scrollback_parser.set_scrollback(usize::MAX);
        let parser_scrollback = self.scrollback_parser.screen().scrollback();
        self.scrollback_parser
            .set_scrollback(current_scrollback.min(parser_scrollback));
        self.max_scrollback = if self.uses_agent_row_history() {
            self.agent_row_history.len()
        } else {
            parser_scrollback
        };
        self.agent_scrollback = self.agent_scrollback.min(self.max_scrollback);
    }

    fn uses_agent_row_history(&self) -> bool {
        matches!(
            self.scrollback_strategy,
            ScrollbackStrategy::AgentMemoryBacked
        ) && !self.agent_row_history.is_empty()
    }

    fn snapshot_history_locked(&self) -> bool {
        self.snapshot_cursor.is_some()
    }

    fn append_agent_row_history(&mut self, rows: &[Vec<u8>]) {
        if !matches!(
            self.scrollback_strategy,
            ScrollbackStrategy::AgentMemoryBacked
        ) || rows.is_empty()
        {
            return;
        }

        for row in rows {
            self.agent_row_history.push_back(row.clone());
        }
        while self.agent_row_history.len() > AGENT_ROW_SCROLLBACK_CAPACITY {
            self.agent_row_history.pop_front();
            self.agent_scrollback = self.agent_scrollback.saturating_sub(1);
        }
    }

    fn agent_row_history_parser(&self) -> vt100::Parser {
        let mut parser = vt100::Parser::new(self.rows, self.cols, 0);
        parser.process(b"\x1b[2J\x1b[H");

        let current_rows: Vec<Vec<u8>> =
            self.parser.screen().rows_formatted(0, self.cols).collect();
        let viewport_rows = current_rows.len();
        let total_rows = self.agent_row_history.len().saturating_add(viewport_rows);
        let top_index = total_rows
            .saturating_sub(viewport_rows)
            .saturating_sub(self.agent_scrollback.min(self.agent_row_history.len()));

        for visible_index in 0..viewport_rows {
            let source_index = top_index.saturating_add(visible_index);
            let row = if source_index < self.agent_row_history.len() {
                self.agent_row_history[source_index].clone()
            } else {
                current_rows
                    .get(source_index.saturating_sub(self.agent_row_history.len()))
                    .cloned()
                    .unwrap_or_default()
            };
            let mut positioned_row = format!("\x1b[{};1H", visible_index + 1).into_bytes();
            positioned_row.extend_from_slice(&row);
            parser.process(&positioned_row);
        }

        parser
    }

    fn active_snapshot(&self) -> Option<&ScreenSnapshot> {
        if self.snapshot_history_locked() {
            return self
                .snapshot_cursor
                .and_then(|index| self.snapshots.get(index));
        }

        None
    }

    fn has_local_cache_scrollback(&self) -> bool {
        if self.uses_snapshot_scrollback() {
            self.has_snapshot_scrollback()
        } else {
            self.max_scrollback() > 0
        }
    }

    fn local_cache_up_capacity(&self) -> usize {
        if !self.has_local_cache_scrollback() {
            return 0;
        }

        if self.uses_snapshot_scrollback() {
            self.snapshot_position()
        } else {
            self.max_scrollback().saturating_sub(self.scrollback())
        }
    }

    fn local_cache_down_capacity(&self) -> usize {
        if !self.has_local_cache_scrollback() {
            return 0;
        }

        if self.uses_snapshot_scrollback() {
            self.snapshots
                .len()
                .saturating_sub(1)
                .saturating_sub(self.snapshot_position())
        } else {
            self.scrollback()
        }
    }

    fn scroll_local_cache_up(&mut self, rows: usize) -> bool {
        if rows == 0 || !self.has_local_cache_scrollback() {
            return false;
        }

        if self.uses_snapshot_scrollback() {
            return self.scroll_snapshot_up(rows);
        }

        let next = self
            .scrollback()
            .saturating_add(rows)
            .min(self.max_scrollback());
        if next == self.scrollback() {
            return false;
        }
        self.set_follow_live(false);
        self.set_scrollback(next);
        true
    }

    fn scroll_local_cache_down(&mut self, rows: usize) -> bool {
        if rows == 0 || !self.has_local_cache_scrollback() {
            return false;
        }

        if self.uses_snapshot_scrollback() {
            return self.scroll_snapshot_down(rows);
        }

        let next = self.scrollback().saturating_sub(rows);
        if next == self.scrollback() {
            return false;
        }
        self.set_scrollback(next);
        self.set_follow_live(next == 0);
        true
    }

    fn scroll_viewport_up(&mut self, rows: usize) -> bool {
        if rows == 0 {
            return false;
        }

        self.scroll_local_cache_up(rows.min(self.local_cache_up_capacity()))
    }

    fn scroll_viewport_down(&mut self, rows: usize) -> bool {
        if rows == 0 {
            return false;
        }

        self.scroll_local_cache_down(rows.min(self.local_cache_down_capacity()))
    }

    fn capture_snapshot(&mut self) -> SnapshotCaptureOutcome {
        let snapshot_count_before = self.snapshots.len();
        let should_capture = match self.scrollback_strategy {
            ScrollbackStrategy::Standard => self.uses_snapshot_scrollback(),
            ScrollbackStrategy::AgentMemoryBacked => true,
        };
        if !should_capture {
            return SnapshotCaptureOutcome::skipped(snapshot_count_before);
        }

        let snapshot = ScreenSnapshot::from_screen(self.rows, self.cols, self.parser.screen());
        let surface_digest = visible_surface_digest(&snapshot.visible_lines);
        let top_preview = first_non_blank_preview(&snapshot.visible_lines);
        let bottom_preview = last_non_blank_preview(&snapshot.visible_lines);

        if self
            .snapshots
            .back()
            .is_some_and(|existing| existing.same_visible_surface(&snapshot))
        {
            return SnapshotCaptureOutcome {
                attempted: true,
                appended: false,
                deduped: true,
                pruned_blank_prefix: 0,
                snapshot_count_after: self.snapshots.len(),
                synthetic_rows_appended: 0,
                surface_digest,
                top_preview,
                bottom_preview,
            };
        }

        self.snapshots.push_back(snapshot);
        if self.snapshots.len() > SNAPSHOT_HISTORY_CAPACITY {
            self.snapshots.pop_front();
            if let Some(cursor) = self.snapshot_cursor {
                self.snapshot_cursor = Some(cursor.saturating_sub(1));
            }
        }
        let pruned_blank_prefix = self.prune_leading_blank_snapshots();
        SnapshotCaptureOutcome {
            attempted: true,
            appended: true,
            deduped: false,
            pruned_blank_prefix,
            snapshot_count_after: self.snapshots.len(),
            synthetic_rows_appended: 0,
            surface_digest,
            top_preview,
            bottom_preview,
        }
    }

    fn prune_leading_blank_snapshots(&mut self) -> usize {
        let mut pruned = 0;
        while self.snapshots.len() > 1
            && self.snapshots.front().is_some_and(ScreenSnapshot::is_blank)
        {
            self.snapshots.pop_front();
            pruned += 1;
            if let Some(cursor) = self.snapshot_cursor {
                self.snapshot_cursor = Some(cursor.saturating_sub(1));
            }
        }
        pruned
    }

    fn log_process_debug(
        &self,
        bytes: &[u8],
        previous_max_scrollback: usize,
        previous_snapshot_count: usize,
        snapshot_outcome: &SnapshotCaptureOutcome,
    ) {
        let strategy = match self.scrollback_strategy {
            ScrollbackStrategy::Standard => "standard",
            ScrollbackStrategy::AgentMemoryBacked => "agent",
        };
        let clear_home_count = count_subslice(bytes, b"\x1b[2J\x1b[H");
        let home_count = count_subslice(bytes, b"\x1b[H");
        let alt_enter_count = count_subslice(bytes, b"\x1b[?1049h");
        let alt_leave_count = count_subslice(bytes, b"\x1b[?1049l");
        crate::scroll_debug::log_lazy(|| {
            format!(
            "event=vt_process strategy={} bytes={} clear_home_count={} home_count={} alt_enter_count={} alt_leave_count={} previous_max_scrollback={} next_max_scrollback={} previous_snapshot_count={} next_snapshot_count={} snapshot_attempted={} snapshot_appended={} snapshot_deduped={} pruned_blank_prefix={} synthetic_rows_appended={} uses_snapshot_scrollback={} surface_digest={} top_preview={} bottom_preview={}",
            strategy,
            bytes.len(),
            clear_home_count,
            home_count,
            alt_enter_count,
            alt_leave_count,
            previous_max_scrollback,
            self.max_scrollback,
            previous_snapshot_count,
            snapshot_outcome.snapshot_count_after,
            snapshot_outcome.attempted,
            snapshot_outcome.appended,
            snapshot_outcome.deduped,
            snapshot_outcome.pruned_blank_prefix,
            snapshot_outcome.synthetic_rows_appended,
            self.uses_snapshot_scrollback(),
            snapshot_outcome.surface_digest,
            snapshot_outcome.top_preview,
            snapshot_outcome.bottom_preview,
        )
        });
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
    /// Specs screen state (SPEC-12 Phase 9 — GitHub Issue SPEC list).
    pub(crate) specs: crate::screens::specs::SpecsState,
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
    /// Branch Cleanup confirm modal (FR-018e).
    pub(crate) cleanup_confirm: crate::screens::cleanup_confirm::CleanupConfirmState,
    /// Branch Cleanup progress modal (FR-018g/h).
    pub(crate) cleanup_progress: crate::screens::cleanup_progress::CleanupProgressState,
    /// Background queue for cleanup runner events.
    pub(crate) cleanup_events: Option<CleanupEventQueue>,
    /// Background channel for merge-state computation events (FR-018d).
    pub(crate) merge_state_events: Option<MergeStateChannel>,
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
            specs: {
                let cache_root = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".gwt")
                    .join("cache")
                    .join("issues");
                crate::screens::specs::SpecsState::new(cache_root)
            },
            wizard: None,
            docker_progress: None,
            docker_progress_events: None,
            branch_detail_worker: None,
            #[cfg(test)]
            branch_detail_docker_snapshotter: None,
            service_select: None,
            port_select: None,
            confirm: ConfirmState::default(),
            cleanup_confirm: crate::screens::cleanup_confirm::CleanupConfirmState::default(),
            cleanup_progress: crate::screens::cleanup_progress::CleanupProgressState::default(),
            cleanup_events: None,
            merge_state_events: None,
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
    /// Public clone of the notification bus sender. Used by `index_worker`
    /// to publish lifecycle events into the Logs tab.
    pub fn notification_bus_handle(&self) -> NotificationBus {
        self._notification_bus.clone()
    }

    /// Repository root currently driving the workspace shell.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Absolute paths of every Worktree currently known to the model.
    /// Used by the index worker bootstrap to spawn watchers and reconcile
    /// orphan index directories.
    pub fn active_worktree_paths(&self) -> Vec<std::path::PathBuf> {
        self.branches
            .branches
            .iter()
            .filter_map(|b| b.worktree_path.clone())
            .collect()
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
                "Branches", "Issues", "PRs", "Specs", "Profiles", "Git View", "Versions",
                "Settings", "Logs",
            ]
        );
        assert_eq!(ManagementTab::Settings.label(), "Settings");
        assert_eq!(ManagementTab::Logs.label(), "Logs");
    }

    #[test]
    fn management_tab_all_has_nine_entries() {
        // SPEC-12 Phase 9: Specs tab raises the count from 8 to 9.
        assert_eq!(ManagementTab::ALL.len(), 9);
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

    fn home_repaint_frame(lines: &[&str]) -> Vec<u8> {
        let mut sequence = String::from("\u{1b}[H");
        for (index, line) in lines.iter().enumerate() {
            sequence.push_str(&format!("\u{1b}[{};1H{}", index + 1, line));
        }
        sequence.into_bytes()
    }

    fn colored_scrollback_lines(lines: &[(&str, u8)]) -> Vec<u8> {
        let mut sequence = String::new();
        for (line, color) in lines {
            sequence.push_str(&format!("\u{1b}[38;5;{color}m{line}\u{1b}[0m\r\n"));
        }
        sequence.into_bytes()
    }

    #[test]
    fn filter_scrollback_bytes_strips_split_alt_screen_sequence() {
        let mut pending = Vec::new();

        let first = filter_scrollback_bytes_with_pending(&mut pending, b"\x1b[?104");
        assert!(first.is_empty());
        assert_eq!(pending, b"\x1b[?104");

        let second = filter_scrollback_bytes_with_pending(&mut pending, b"9hhello");
        assert_eq!(second, b"hello");
        assert!(pending.is_empty());
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
    fn agent_scrollback_strategy_prefers_normalized_row_history_when_available() {
        let mut vt = VtState::new(4, 20);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(b"\x1b[?1049h\x1b[2J\x1b[Hlaunch");
        vt.process(b"\x1b[2J\x1b[H");
        vt.process(b"line-1\r\nline-2\r\nline-3\r\nline-4\r\nline-5\r\nline-6");

        assert!(vt.max_scrollback() > 0);
        assert!(!vt.uses_snapshot_scrollback());

        assert!(vt.scroll_viewport_lines(vt.max_scrollback() as i16));
        let contents = vt.visible_screen_parser().screen().contents();
        assert!(contents.contains("line-1"));
    }

    #[test]
    fn agent_scrollback_strategy_falls_back_to_snapshot_history_when_rows_do_not_advance() {
        let mut vt = VtState::new(5, 20);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(&full_screen_frame(&[
            "alpha-1", "alpha-2", "alpha-3", "alpha-4", "alpha-5",
        ]));
        vt.process(&full_screen_frame(&[
            "beta-1", "beta-2", "beta-3", "beta-4", "beta-5",
        ]));

        assert_eq!(vt.max_scrollback(), 0);
        assert!(vt.has_snapshot_scrollback());
        assert!(vt.uses_snapshot_scrollback());

        assert!(vt.scroll_viewport_lines(1));
        let contents = vt.visible_screen_parser().screen().contents();
        assert!(contents.contains("alpha-1"));
        assert!(!contents.contains("beta-1"));
    }

    #[test]
    fn agent_scrollback_strategy_keeps_intermediate_full_screen_frames_within_one_payload() {
        let mut vt = VtState::new(5, 20);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        let mut payload = b"\x1b[?1049h".to_vec();
        payload.extend_from_slice(&full_screen_frame(&[
            "alpha-1", "alpha-2", "alpha-3", "alpha-4", "alpha-5",
        ]));
        payload.extend_from_slice(&full_screen_frame(&[
            "beta-1", "beta-2", "beta-3", "beta-4", "beta-5",
        ]));

        vt.process(&payload);

        assert!(
            vt.has_snapshot_scrollback(),
            "coalesced PTY payloads should still preserve older full-screen frames for agent scrollback"
        );
        assert!(vt.scroll_viewport_lines(1));

        let contents = vt.visible_screen_parser().screen().contents();
        assert!(contents.contains("alpha-1"));
        assert!(!contents.contains("beta-1"));
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
                formatted_rows: vec![Vec::new(); 5],
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
                formatted_rows: vec![Vec::new(); 5],
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
                formatted_rows: vec![Vec::new(); 5],
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
                formatted_rows: vec![Vec::new(); 5],
            },
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: vec![2],
                visible_lines: vec!["frame-2".to_string(); 5],
                formatted_rows: vec![Vec::new(); 5],
            },
            ScreenSnapshot {
                rows: 5,
                cols: 20,
                state: vec![3],
                visible_lines: vec!["frame-3".to_string(); 5],
                formatted_rows: vec![Vec::new(); 5],
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

    #[test]
    fn agent_scrollback_strategy_preserves_styles_across_in_memory_history() {
        let mut vt = VtState::new(4, 32);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);
        vt.process(&colored_scrollback_lines(&[
            ("line-1", 196),
            ("line-2", 202),
            ("line-3", 208),
            ("line-4", 214),
            ("line-5", 220),
            ("line-6", 226),
        ]));

        assert!(vt.max_scrollback() > 0);
        assert!(vt.scroll_viewport_lines(vt.max_scrollback() as i16));

        let parser = vt.visible_screen_parser();
        let cell = parser.screen().cell(0, 0).expect("styled cell");
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(196));
        assert!(parser.screen().contents().contains("line-1"));
        assert!(parser.screen().contents().contains("line-4"));
        assert!(!parser.screen().contents().contains("line-6"));
    }

    #[test]
    fn agent_scrollback_strategy_keeps_large_in_memory_row_history() {
        let mut vt = VtState::new(4, 32);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        let mut output = String::new();
        for index in 1..=11_050 {
            if index > 1 {
                output.push_str("\r\n");
            }
            output.push_str(&format!("line-{index:05}"));
        }
        vt.process(output.as_bytes());

        assert!(
            vt.max_scrollback() > ROW_SCROLLBACK_CAPACITY,
            "agent panes should keep a larger in-memory row scrollback than the default terminal history limit"
        );
    }

    #[test]
    fn agent_scrollback_strategy_derives_row_history_from_full_screen_redraw_shifts() {
        let mut vt = VtState::new(5, 20);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(&full_screen_frame(&[
            "\u{1b}[38;5;196mline-1\u{1b}[0m",
            "line-2",
            "line-3",
            "line-4",
            "line-5",
        ]));
        vt.process(&full_screen_frame(&[
            "line-2",
            "line-3",
            "line-4",
            "line-5",
            "\u{1b}[38;5;226mline-6\u{1b}[0m",
        ]));

        assert_eq!(
            vt.max_scrollback(),
            1,
            "a one-line full-screen redraw shift should be promoted into row scrollback history"
        );
        assert!(
            !vt.uses_snapshot_scrollback(),
            "once redraw shifts are normalized into row history, agent panes should stop using frame-by-frame snapshot scrolling"
        );

        assert!(vt.scroll_viewport_lines(1));
        let parser = vt.visible_screen_parser();
        let screen = parser.screen();
        assert!(screen.contents().contains("line-1"));
        assert!(!screen.contents().contains("line-6"));
        let cell = screen.cell(0, 0).expect("styled cell");
        assert_eq!(
            cell.fgcolor(),
            vt100::Color::Idx(196),
            "derived row history should preserve ANSI styling for scrolled-off rows"
        );
    }

    #[test]
    fn agent_scrollback_strategy_derives_row_history_from_home_repaint_shifts() {
        let mut vt = VtState::new(5, 20);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(&full_screen_frame(&[
            "line-1", "line-2", "line-3", "line-4", "line-5",
        ]));
        vt.process(&home_repaint_frame(&[
            "line-2", "line-3", "line-4", "line-5", "line-6",
        ]));

        assert_eq!(
            vt.max_scrollback(),
            1,
            "home-only full-screen repaints should also promote scrolled-off rows into row history"
        );
        assert!(
            !vt.uses_snapshot_scrollback(),
            "once a vertical repaint shift is normalized into row history, Codex-like panes should stop stepping snapshots"
        );

        assert!(vt.scroll_viewport_lines(1));
        let contents = vt.visible_screen_parser().screen().contents();
        assert!(contents.contains("line-1"));
        assert!(!contents.contains("line-6"));
    }

    #[test]
    fn agent_scrollback_strategy_derives_row_history_from_status_churn_shift() {
        let mut vt = VtState::new(5, 24);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(&full_screen_frame(&[
            "status-a", "line-1", "line-2", "line-3", "line-4",
        ]));
        vt.process(&home_repaint_frame(&[
            "status-b", "line-2", "line-3", "line-4", "line-5",
        ]));

        assert_eq!(
            vt.max_scrollback(),
            1,
            "one-row vertical shifts should still become row history even when one leading status row changes during the redraw"
        );
        assert!(
            !vt.uses_snapshot_scrollback(),
            "status churn should not force Codex-like panes back into page-sized snapshot stepping"
        );

        assert!(vt.scroll_viewport_lines(1));
        let contents = vt.visible_screen_parser().screen().contents();
        assert!(contents.contains("line-1"));
        assert!(!contents.contains("status-a"));
        assert!(!contents.contains("line-5"));
    }

    #[test]
    fn agent_scrollback_strategy_derives_row_history_from_sparse_shift_matches() {
        let mut vt = VtState::new(6, 24);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(&full_screen_frame(&[
            "status-a", "line-1", "line-2", "line-3", "line-4", "line-5",
        ]));
        vt.process(&home_repaint_frame(&[
            "status-b", "line-2", "progress", "line-4", "spinner", "line-6",
        ]));

        assert_eq!(
            vt.max_scrollback(),
            1,
            "Codex-like redraws should still derive one line of row history when the vertical shift is visible through sparse same-offset matches"
        );
        assert!(
            !vt.uses_snapshot_scrollback(),
            "sparse overlap should not force the pane back into page-sized snapshot stepping"
        );

        assert!(vt.scroll_viewport_lines(1));
        let contents = vt.visible_screen_parser().screen().contents();
        assert!(contents.contains("line-1"));
        assert!(!contents.contains("line-6"));
    }

    #[test]
    fn agent_snapshot_history_stays_frozen_while_new_row_history_arrives() {
        let mut vt = VtState::new(5, 20);
        vt.set_scrollback_strategy(ScrollbackStrategy::AgentMemoryBacked);

        vt.process(&full_screen_frame(&[
            "alpha-1", "alpha-2", "alpha-3", "alpha-4", "alpha-5",
        ]));
        vt.process(&full_screen_frame(&[
            "beta-1", "beta-2", "beta-3", "beta-4", "beta-5",
        ]));

        assert!(vt.uses_snapshot_scrollback());
        assert!(vt.scroll_viewport_lines(1));
        assert_eq!(vt.snapshot_position(), 0);

        vt.process(&full_screen_frame(&[
            "beta-2", "beta-3", "beta-4", "beta-5", "gamma-6",
        ]));

        assert_eq!(
            vt.max_scrollback(),
            1,
            "incoming PTY redraws should still be normalized into row history in the background"
        );
        assert_eq!(
            vt.snapshot_position(),
            0,
            "while browsing snapshot history, the cursor should stay anchored to the same frame"
        );

        let contents = vt.visible_screen_parser().screen().contents();
        assert!(
            contents.contains("alpha-1"),
            "new row history must not replace the snapshot the user is currently viewing"
        );
        assert!(
            !contents.contains("gamma-6"),
            "live redraws should stay hidden until the user explicitly returns to live"
        );
    }

    #[test]
    fn mouse_reporting_state_tracks_split_enable_and_disable_sequences() {
        let mut vt = VtState::new(4, 32);

        vt.process(b"\x1b[?100");
        assert!(!vt.accepts_mouse_scroll_input());

        vt.process(b"0h\x1b[?100");
        assert!(!vt.accepts_mouse_scroll_input());

        vt.process(b"6h");
        assert!(
            vt.accepts_mouse_scroll_input(),
            "mouse wheel forwarding should enable once both report mode and SGR encoding are active"
        );

        vt.process(b"\x1b[?1000l");
        assert!(
            !vt.accepts_mouse_scroll_input(),
            "disabling button tracking should disable forwarded mouse scroll input"
        );
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
