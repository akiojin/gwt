//! Branches management screen.

use std::collections::{HashMap, HashSet};

use gwt_agent::AgentStatus;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::theme;

/// Sort mode for the branch list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Default,
    Name,
    Date,
}

impl SortMode {
    /// Cycle to the next sort mode.
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Name,
            Self::Name => Self::Date,
            Self::Date => Self::Default,
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Name => "Name",
            Self::Date => "Date",
        }
    }
}

/// View mode filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    All,
    #[default]
    Local,
    Remote,
}

impl ViewMode {
    /// Cycle to the next view mode.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Local,
            Self::Local => Self::Remote,
            Self::Remote => Self::All,
        }
    }

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Local => "Local",
            Self::Remote => "Remote",
        }
    }
}

/// Branch category derived from name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BranchCategory {
    Main,
    Develop,
    Feature,
    Other,
}

impl BranchCategory {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "Main",
            Self::Develop => "Develop",
            Self::Feature => "Feature",
            Self::Other => "Other",
        }
    }
}

/// Categorize a branch by its name.
pub fn categorize_branch(name: &str) -> BranchCategory {
    let base = name.strip_prefix("origin/").unwrap_or(name);
    if base == "main" || base == "master" {
        BranchCategory::Main
    } else if base == "develop" || base == "development" {
        BranchCategory::Develop
    } else if base.starts_with("feature/") || base.starts_with("feat/") {
        BranchCategory::Feature
    } else {
        BranchCategory::Other
    }
}

/// A single branch entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchItem {
    pub name: String,
    pub is_head: bool,
    pub is_local: bool,
    pub category: BranchCategory,
    pub worktree_path: Option<std::path::PathBuf>,
}

/// Per-branch merge state used by Branch Cleanup (FR-018a/d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeState {
    /// Background merge detection has not produced a result yet.
    Computing,
    /// Branch is fully merged into the named target and can be cleaned up.
    Cleanable(gwt_git::MergeTarget),
    /// Branch is not merged into any configured base.
    NotMerged,
}

/// User-configurable Branch Cleanup settings (FR-018e).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupSettings {
    /// Whether the next confirmed cleanup should also delete the matching
    /// remote tracking branch via `gh` / `git push --delete origin`.
    pub delete_remote: bool,
}

/// Phase of an in-flight Branch Cleanup execution (FR-018g/h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupRunPhase {
    Running,
    Done,
}

/// Outcome row for a single branch in a Cleanup run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupResultRow {
    pub branch: String,
    pub success: bool,
    pub message: Option<String>,
}

/// Live state of a Branch Cleanup execution displayed in the progress modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupRunState {
    pub total: usize,
    pub processed: usize,
    pub current: Option<String>,
    pub results: Vec<CleanupResultRow>,
    pub phase: CleanupRunPhase,
    pub delete_remote: bool,
}

impl CleanupRunState {
    pub fn new(total: usize, delete_remote: bool) -> Self {
        Self {
            total,
            processed: 0,
            current: None,
            results: Vec::with_capacity(total),
            phase: CleanupRunPhase::Running,
            delete_remote,
        }
    }

    /// Append a per-branch result, advance `processed`, and clear `current`.
    pub fn record_result(&mut self, row: CleanupResultRow) {
        self.results.push(row);
        self.processed = self.processed.saturating_add(1);
        self.current = None;
    }

    /// Mark the run as finished. Idempotent.
    pub fn finish(&mut self) {
        self.phase = CleanupRunPhase::Done;
        self.current = None;
    }

    pub fn succeeded(&self) -> usize {
        self.results.iter().filter(|r| r.success).count()
    }

    pub fn failed(&self) -> usize {
        self.results.iter().filter(|r| !r.success).count()
    }
}

/// A SPEC entry loaded from a branch worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailSpecItem {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub status: String,
}

/// A lightweight summary of an active session associated with the selected branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailSessionSummary {
    pub kind: &'static str,
    pub name: String,
    pub detail: Option<String>,
    pub active: bool,
}

/// Lightweight live session summary rendered directly in the branch list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchLiveSessionSummary {
    pub indicators: Vec<BranchLiveSessionIndicator>,
}

/// Lightweight live session indicator rendered directly in the branch list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BranchLiveSessionIndicator {
    pub status: AgentStatus,
    pub color: crate::model::AgentColor,
}

/// Lifecycle action requested for a Docker container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerLifecycleAction {
    Start,
    Stop,
    Restart,
}

/// Pending Docker action selected in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDockerAction {
    pub container_id: String,
    pub action: DockerLifecycleAction,
}

/// Cached detail payload for a branch.
#[derive(Debug, Clone, Default)]
pub struct BranchDetailData {
    pub specs: Vec<DetailSpecItem>,
    pub files: Vec<String>,
    pub commits: Vec<String>,
    pub docker_containers: Vec<gwt_docker::ContainerInfo>,
}

/// Background detail-load result for a single branch.
#[derive(Debug, Clone)]
pub struct BranchDetailLoadResult {
    pub generation: u64,
    pub branch_name: String,
    pub data: BranchDetailData,
}

/// Number of detail sections in the branch detail view.
const DETAIL_SECTION_COUNT: usize = 4;

/// Labels for the detail sections.
const DETAIL_SECTION_LABELS: [&str; DETAIL_SECTION_COUNT] =
    ["Overview", "SPECs", "Git", "Sessions"];

/// Public accessor for detail section labels (used by app.rs for pane titles).
pub fn detail_section_labels() -> &'static [&'static str] {
    &DETAIL_SECTION_LABELS
}

/// State for the branches screen.
#[derive(Debug, Clone, Default)]
pub struct BranchesState {
    pub(crate) branches: Vec<BranchItem>,
    pub(crate) selected: usize,
    pub(crate) sort_mode: SortMode,
    pub(crate) view_mode: ViewMode,
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    /// Active detail section: 0=Overview, 1=SPECs, 2=Git, 3=Sessions.
    pub(crate) detail_section: usize,
    /// Selected row within the Sessions detail section.
    pub(crate) detail_session_selected: usize,
    /// Flag: caller should open agent selection.
    pub(crate) pending_launch_agent: bool,
    /// Flag: caller should spawn shell in worktree cwd.
    pub(crate) pending_open_shell: bool,
    /// Branch Cleanup multi-selection set (FR-018c). Names are kept stable
    /// across view-mode/sort/search/tab changes and only cleared after a
    /// cleanup run completes.
    pub(crate) cleanup_selected: HashSet<String>,
    /// Per-branch merge state used by Branch Cleanup (FR-018a/d).
    pub(crate) merged_state: HashMap<String, MergeState>,
    /// Persisted Branch Cleanup settings (FR-018e).
    /// Wired into the confirm modal in Phase 51.4.
    #[allow(dead_code)]
    pub(crate) cleanup_settings: CleanupSettings,
    /// Active Branch Cleanup run, when present (FR-018g/h).
    /// Wired into the progress modal in Phase 51.4.
    #[allow(dead_code)]
    pub(crate) cleanup_run: Option<CleanupRunState>,
    /// Branches that should never be selected because another worktree owns
    /// them. Updated by app.rs from the live worktree list.
    pub(crate) checked_out_branches: HashSet<String>,
    /// Branches with at least one running session pane bound to them.
    pub(crate) active_session_branches: HashSet<String>,
    /// HEAD branch of the gwt-tui process itself, if known.
    pub(crate) current_head_branch: Option<String>,
    /// Spinner frame counter advanced once per tick while at least one
    /// branch is still in `MergeState::Computing`. Drives the animated
    /// `⋯` glyph in the Branch Cleanup gutter.
    pub(crate) merge_spinner_tick: usize,
    /// SPECs loaded from the selected branch worktree.
    pub(crate) detail_specs: Vec<DetailSpecItem>,
    /// Git status files for the selected branch worktree.
    pub(crate) detail_files: Vec<String>,
    /// Recent commits for the selected branch worktree.
    pub(crate) detail_commits: Vec<String>,
    /// Docker containers available for the selected branch context.
    pub(crate) docker_containers: Vec<gwt_docker::ContainerInfo>,
    /// Selected Docker container index in the overview area.
    pub(crate) docker_selected: usize,
    /// Pending Docker action intent to be handled by the caller.
    pub(crate) pending_docker_action: Option<PendingDockerAction>,
    /// Cached branch detail payloads keyed by branch name.
    pub(crate) detail_cache: HashMap<String, BranchDetailData>,
    /// Branches currently being loaded in the background.
    pub(crate) loading_branches: HashSet<String>,
    /// Monotonic generation used to discard stale async detail results.
    pub(crate) detail_generation: u64,
    /// Hook-derived live session summaries keyed by branch name.
    pub(crate) live_session_summaries: HashMap<String, BranchLiveSessionSummary>,
    /// Tick-driven animation counter for branch-row running indicators.
    pub(crate) session_animation_tick: usize,
}

impl BranchesState {
    /// Return branches filtered by current view mode and search query,
    /// then sorted according to the active `sort_mode`.
    pub fn filtered_branches(&self) -> Vec<&BranchItem> {
        let query_lower = self.search_query.to_lowercase();
        let mut result: Vec<&BranchItem> = self
            .branches
            .iter()
            .filter(|b| match self.view_mode {
                ViewMode::All => true,
                ViewMode::Local => b.is_local,
                ViewMode::Remote => !b.is_local,
            })
            .filter(|b| query_lower.is_empty() || b.name.to_lowercase().contains(&query_lower))
            .collect();

        match self.sort_mode {
            SortMode::Default => {
                if self.view_mode == ViewMode::All {
                    let (mut local, remote): (Vec<&BranchItem>, Vec<&BranchItem>) =
                        result.into_iter().partition(|branch| branch.is_local);
                    local.extend(remote);
                    result = local;
                }
            }
            // Date has no dedicated field yet; fall back to alphabetical like Name.
            SortMode::Name | SortMode::Date => result.sort_by(|a, b| {
                if self.view_mode == ViewMode::All {
                    b.is_local
                        .cmp(&a.is_local)
                        .then_with(|| a.name.cmp(&b.name))
                } else {
                    a.name.cmp(&b.name)
                }
            }),
        }

        result
    }

    /// Get the currently selected branch (from filtered list).
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        let filtered = self.filtered_branches();
        filtered.get(self.selected).copied()
    }

    /// Clamp selected index to filtered length.
    fn clamp_selected(&mut self) {
        let len = self.filtered_branches().len();
        super::clamp_index(&mut self.selected, len);
    }

    /// Clamp selected Docker container index to available containers.
    fn clamp_docker_selected(&mut self) {
        let len = self.docker_containers.len();
        super::clamp_index(&mut self.docker_selected, len);
    }

    /// Clamp the session-row selection for the Sessions detail section.
    pub(crate) fn clamp_detail_session_selected(&mut self, len: usize) {
        super::clamp_index(&mut self.detail_session_selected, len);
    }

    /// Return the currently selected Docker container, if any.
    fn selected_docker_container(&self) -> Option<&gwt_docker::ContainerInfo> {
        self.docker_containers.get(self.docker_selected)
    }

    fn selected_branch_name(&self) -> Option<String> {
        self.selected_branch().map(|branch| branch.name.clone())
    }

    fn apply_detail_data(&mut self, data: &BranchDetailData, reset_docker_selection: bool) {
        self.detail_specs = data.specs.clone();
        self.detail_files = data.files.clone();
        self.detail_commits = data.commits.clone();
        self.docker_containers = data.docker_containers.clone();
        if reset_docker_selection {
            self.docker_selected = 0;
        }
        self.clamp_docker_selected();
        self.pending_docker_action = None;
    }

    fn clear_visible_detail(&mut self) {
        self.detail_specs.clear();
        self.detail_files.clear();
        self.detail_commits.clear();
        self.docker_containers.clear();
        self.docker_selected = 0;
        self.pending_docker_action = None;
    }

    fn sync_selected_detail_from_cache(&mut self, reset_docker_selection: bool) {
        let Some(branch_name) = self.selected_branch_name() else {
            self.clear_visible_detail();
            return;
        };

        let cached = self.detail_cache.get(&branch_name).cloned();
        if let Some(data) = cached {
            self.apply_detail_data(&data, reset_docker_selection);
        } else {
            self.clear_visible_detail();
        }
    }

    fn prune_detail_cache(&mut self) {
        let branch_names: HashSet<String> = self
            .branches
            .iter()
            .map(|branch| branch.name.clone())
            .collect();
        self.detail_cache
            .retain(|branch_name, _| branch_names.contains(branch_name));
        self.loading_branches
            .retain(|branch_name| branch_names.contains(branch_name));
    }

    fn worktree_sources(&self) -> HashMap<String, Option<std::path::PathBuf>> {
        self.branches
            .iter()
            .map(|branch| (branch.name.clone(), branch.worktree_path.clone()))
            .collect()
    }

    fn evict_changed_detail_sources(
        &mut self,
        previous_sources: &HashMap<String, Option<std::path::PathBuf>>,
    ) {
        for branch in &self.branches {
            if previous_sources
                .get(&branch.name)
                .is_some_and(|previous| previous == &branch.worktree_path)
            {
                continue;
            }
            self.detail_cache.remove(&branch.name);
            self.loading_branches.remove(&branch.name);
        }
    }

    pub(crate) fn begin_detail_refresh(&mut self) -> (u64, Vec<BranchItem>) {
        self.detail_generation = self.detail_generation.wrapping_add(1);
        self.loading_branches = self
            .branches
            .iter()
            .map(|branch| branch.name.clone())
            .collect();
        self.sync_selected_detail_from_cache(false);
        (self.detail_generation, self.branches.clone())
    }

    pub(crate) fn cache_detail(&mut self, branch_name: String, data: BranchDetailData) {
        let selected_branch = self.selected_branch_name();
        let is_selected = selected_branch.as_deref() == Some(branch_name.as_str());
        self.loading_branches.remove(&branch_name);
        self.detail_cache.insert(branch_name, data.clone());
        if is_selected {
            self.apply_detail_data(&data, false);
        }
    }

    pub(crate) fn selected_detail_loading(&self) -> bool {
        let Some(branch_name) = self.selected_branch_name() else {
            return false;
        };
        self.loading_branches.contains(&branch_name)
            && !self.detail_cache.contains_key(&branch_name)
    }

    pub(crate) fn knows_branch(&self, branch_name: &str) -> bool {
        self.branches
            .iter()
            .any(|branch| branch.name == branch_name)
    }

    // ---- Branch Cleanup helpers (FR-018) ----

    /// Replace the merge state for a single branch (background preload sink).
    pub fn set_merge_state(&mut self, branch: impl Into<String>, state: MergeState) {
        self.merged_state.insert(branch.into(), state);
    }

    /// Returns the merge state for `branch`, defaulting to `Computing` so the
    /// list view can render the spinner glyph until a result lands.
    pub fn merge_state(&self, branch: &str) -> MergeState {
        self.merged_state
            .get(branch)
            .copied()
            .unwrap_or(MergeState::Computing)
    }

    /// Returns true when the branch is currently selected for cleanup.
    pub fn is_cleanup_selected(&self, branch: &str) -> bool {
        self.cleanup_selected.contains(branch)
    }

    /// Returns true when `branch` is allowed to participate in Branch Cleanup
    /// selection (FR-018b).
    ///
    /// Protected names, the current HEAD, and branches with an active agent
    /// or shell session are rejected. Feature branches that are checked out
    /// by a worktree **are still candidates** — cleaning up the worktree is
    /// the whole point of Branch Cleanup in a gwt workflow where every
    /// branch normally has its own worktree.
    pub fn is_cleanable_candidate(&self, branch: &str) -> bool {
        if gwt_git::is_protected_branch(branch) {
            return false;
        }
        if self
            .current_head_branch
            .as_deref()
            .is_some_and(|head| head == branch)
        {
            return false;
        }
        if self.active_session_branches.contains(branch) {
            return false;
        }
        true
    }

    /// Returns true when the branch is both a protection-allowed candidate
    /// and has a positive merge result that makes it cleanable right now.
    pub fn is_cleanup_selectable(&self, branch: &str) -> bool {
        if !self.is_cleanable_candidate(branch) {
            return false;
        }
        matches!(self.merge_state(branch), MergeState::Cleanable(_))
    }

    /// Toggle Branch Cleanup selection for `branch`. No-op when the branch
    /// is not currently selectable (FR-018c).
    pub fn toggle_cleanup_selection(&mut self, branch: &str) -> bool {
        if !self.is_cleanup_selectable(branch) {
            return false;
        }
        if self.cleanup_selected.contains(branch) {
            self.cleanup_selected.remove(branch);
        } else {
            self.cleanup_selected.insert(branch.to_string());
        }
        true
    }

    /// Add every currently visible cleanable branch to the selection set.
    /// `visible_branches` should come from `filtered_branches()` so that the
    /// search query / view mode / sort filter is honored.
    pub fn select_all_visible_cleanable(&mut self, visible_branches: &[&BranchItem]) {
        let names: Vec<String> = visible_branches
            .iter()
            .filter(|branch| self.is_cleanup_selectable(&branch.name))
            .map(|branch| branch.name.clone())
            .collect();
        for name in names {
            self.cleanup_selected.insert(name);
        }
    }

    /// Cleanup execution finished — drop the selection set so the next
    /// browse session starts clean. Called only after a run completes.
    pub fn clear_selection_after_cleanup(&mut self) {
        self.cleanup_selected.clear();
    }

    /// Number of currently selected branches.
    pub fn cleanup_selection_count(&self) -> usize {
        self.cleanup_selected.len()
    }

    /// Returns the merge target for a branch when it is cleanable.
    pub fn cleanup_target(&self, branch: &str) -> Option<gwt_git::MergeTarget> {
        match self.merge_state(branch) {
            MergeState::Cleanable(target) => Some(target),
            _ => None,
        }
    }

    /// Returns true when at least one branch is still being computed by
    /// the background merge-state worker.
    pub fn has_computing_branches(&self) -> bool {
        self.merged_state
            .values()
            .any(|state| matches!(state, MergeState::Computing))
    }

    /// Advance the spinner frame counter. Called from the tick loop only
    /// while `has_computing_branches()` is true.
    pub fn tick_merge_spinner(&mut self) {
        self.merge_spinner_tick = self.merge_spinner_tick.wrapping_add(1);
    }

    /// Current spinner glyph for the `⋯` gutter slot. Cycles through a
    /// short Braille pattern so the gutter visibly animates while merge
    /// detection is running.
    pub fn merge_spinner_glyph(&self) -> &'static str {
        const FRAMES: [&str; 10] = [
            "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}",
            "\u{2827}", "\u{2807}", "\u{280F}",
        ];
        FRAMES[self.merge_spinner_tick % FRAMES.len()]
    }
}

/// Messages specific to the branches screen.
#[derive(Debug, Clone)]
pub enum BranchesMessage {
    MoveUp,
    MoveDown,
    Select,
    ToggleSort,
    ToggleView,
    SearchStart,
    SearchInput(char),
    SearchBackspace,
    SearchClear,
    Refresh,
    SetBranches(Vec<BranchItem>),
    /// Cycle to the next detail section.
    NextDetailSection,
    /// Cycle to the previous detail section.
    PrevDetailSection,
    /// Launch agent action.
    LaunchAgent,
    /// Open shell action.
    OpenShell,
    /// Move to the next Docker container in the overview area.
    DockerContainerDown,
    /// Move to the previous Docker container in the overview area.
    DockerContainerUp,
    /// Request a start lifecycle action for the selected Docker container.
    DockerContainerStart,
    /// Request a stop lifecycle action for the selected Docker container.
    DockerContainerStop,
    /// Request a restart lifecycle action for the selected Docker container.
    DockerContainerRestart,
}

/// Update branches state in response to a message.
pub fn update(state: &mut BranchesState, msg: BranchesMessage) {
    match msg {
        BranchesMessage::MoveUp => {
            let len = state.filtered_branches().len();
            super::clamp_index(&mut state.selected, len);
            state.selected = state.selected.saturating_sub(1);
            state.detail_session_selected = 0;
            state.sync_selected_detail_from_cache(true);
        }
        BranchesMessage::MoveDown => {
            let len = state.filtered_branches().len();
            super::clamp_index(&mut state.selected, len);
            if len > 0 && state.selected + 1 < len {
                state.selected += 1;
            }
            state.detail_session_selected = 0;
            state.sync_selected_detail_from_cache(true);
        }
        BranchesMessage::Select => {
            if !state.filtered_branches().is_empty() {
                state.pending_launch_agent = true;
            }
        }
        BranchesMessage::ToggleSort => {
            state.sort_mode = state.sort_mode.next();
            state.detail_session_selected = 0;
            state.sync_selected_detail_from_cache(true);
        }
        BranchesMessage::ToggleView => {
            state.view_mode = state.view_mode.next();
            state.clamp_selected();
            state.detail_session_selected = 0;
            state.sync_selected_detail_from_cache(true);
        }
        BranchesMessage::SearchStart => {
            state.search_active = true;
        }
        BranchesMessage::SearchInput(ch) => {
            if state.search_active {
                state.search_query.push(ch);
                state.clamp_selected();
                state.detail_session_selected = 0;
                state.sync_selected_detail_from_cache(true);
            }
        }
        BranchesMessage::SearchBackspace => {
            if state.search_active {
                state.search_query.pop();
                state.clamp_selected();
                state.detail_session_selected = 0;
                state.sync_selected_detail_from_cache(true);
            }
        }
        BranchesMessage::SearchClear => {
            state.search_query.clear();
            state.search_active = false;
            state.clamp_selected();
            state.detail_session_selected = 0;
            state.sync_selected_detail_from_cache(true);
        }
        BranchesMessage::Refresh => {
            // Signal to reload branches — handled by caller
        }
        BranchesMessage::SetBranches(branches) => {
            let previous_sources = state.worktree_sources();
            state.branches = branches;
            state.evict_changed_detail_sources(&previous_sources);
            state.prune_detail_cache();
            state.clamp_selected();
            state.detail_session_selected = 0;
            state.sync_selected_detail_from_cache(true);
        }
        BranchesMessage::NextDetailSection => {
            state.detail_section = (state.detail_section + 1) % DETAIL_SECTION_COUNT;
        }
        BranchesMessage::PrevDetailSection => {
            state.detail_section = if state.detail_section == 0 {
                DETAIL_SECTION_COUNT - 1
            } else {
                state.detail_section - 1
            };
        }
        BranchesMessage::LaunchAgent => {
            state.pending_launch_agent = true;
        }
        BranchesMessage::OpenShell => {
            state.pending_open_shell = true;
        }
        BranchesMessage::DockerContainerDown => {
            if !state.docker_containers.is_empty() {
                super::move_down(&mut state.docker_selected, state.docker_containers.len());
            }
        }
        BranchesMessage::DockerContainerUp => {
            if !state.docker_containers.is_empty() {
                super::move_up(&mut state.docker_selected, state.docker_containers.len());
            }
        }
        BranchesMessage::DockerContainerStart => {
            if let Some(container) = state.selected_docker_container() {
                state.pending_docker_action = Some(PendingDockerAction {
                    container_id: container.id.clone(),
                    action: DockerLifecycleAction::Start,
                });
            }
        }
        BranchesMessage::DockerContainerStop => {
            if let Some(container) = state.selected_docker_container() {
                state.pending_docker_action = Some(PendingDockerAction {
                    container_id: container.id.clone(),
                    action: DockerLifecycleAction::Stop,
                });
            }
        }
        BranchesMessage::DockerContainerRestart => {
            if let Some(container) = state.selected_docker_container() {
                state.pending_docker_action = Some(PendingDockerAction {
                    container_id: container.id.clone(),
                    action: DockerLifecycleAction::Restart,
                });
            }
        }
    }
}

/// Load detail data (SPECs, git status, commits, docker state) for the branch.
///
/// Best-effort: all errors are silently ignored.
pub fn load_branch_detail(
    branch: &BranchItem,
    docker_containers: &[gwt_docker::ContainerInfo],
) -> BranchDetailData {
    let mut detail = BranchDetailData {
        docker_containers: docker_containers.to_vec(),
        ..BranchDetailData::default()
    };

    let Some(wt_path) = branch.worktree_path.clone() else {
        return detail;
    };

    // Load SPECs from worktree specs/ directory
    if let Ok(entries) = std::fs::read_dir(wt_path.join("specs")) {
        let mut specs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !dir_name.starts_with("SPEC-") {
                continue;
            }
            let metadata_path = path.join("metadata.json");
            let Ok(content) = std::fs::read_to_string(&metadata_path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };
            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(dir_name)
                .to_string();
            let title = value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let phase = value
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let status = value
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            specs.push(DetailSpecItem {
                id,
                title,
                phase,
                status,
            });
        }
        specs.sort_by(|a, b| spec_sort_key(&a.id).cmp(&spec_sort_key(&b.id)));
        detail.specs = specs;
    }

    // Load git status
    if let Ok(entries) = gwt_git::diff::get_status(&wt_path) {
        detail.files = entries
            .iter()
            .map(|e| {
                let tag = match e.status {
                    gwt_git::FileStatus::Staged => "[S]",
                    gwt_git::FileStatus::Unstaged => "[U]",
                    gwt_git::FileStatus::Untracked => "[?]",
                };
                format!("{} {}", tag, e.path.display())
            })
            .collect();
    }

    // Load recent commits
    if let Ok(commits) = gwt_git::commit::recent_commits(&wt_path, 5) {
        detail.commits = commits
            .iter()
            .map(|c| format!("{} {}", c.hash, c.subject))
            .collect();
    }

    detail
}

fn spec_sort_key(spec_id: &str) -> (u64, String) {
    let numeric = spec_id
        .strip_prefix("SPEC-")
        .and_then(|suffix| suffix.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    (numeric, spec_id.to_string())
}

/// Which sub-pane of the branches screen is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchesFocus {
    List,
    Detail,
    None,
}

/// Render the branch list pane content (header + list, no borders).
///
/// Called by app.rs which provides the bordered pane. This renders borderless
/// content into the inner area of the top management pane.
pub fn render_list(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let list_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header: view/sort/search (no border)
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    render_header(state, frame, list_chunks[0]);
    render_branch_list(state, frame, list_chunks[1]);
}

/// Render the branch detail content (no borders, no title).
///
/// Called by app.rs which provides the bordered pane. This renders borderless
/// content into the inner area of the bottom detail pane.
/// `sessions` contains branch-scoped active session summaries for this branch.
pub fn render_detail_content(
    state: &BranchesState,
    frame: &mut Frame,
    area: Rect,
    sessions: &[DetailSessionSummary],
) {
    match state.detail_section {
        0 => render_detail_overview(state, frame, area),
        1 => render_detail_specs(state, frame, area),
        2 => render_detail_git_status(state, frame, area),
        3 => render_detail_sessions(frame, area, sessions, state.detail_session_selected),
        _ => {}
    }
}

/// Render the branches screen (legacy entry point).
///
/// In the lazygit layout, app.rs calls render_list / render_detail_content directly.
pub fn render(state: &BranchesState, frame: &mut Frame, area: Rect, _focus: BranchesFocus) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let list_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(main_chunks[0]);

    render_header(state, frame, list_chunks[0]);
    render_branch_list(state, frame, list_chunks[1]);
    render_detail_content(state, frame, main_chunks[1], &[]);
}

/// Render the header bar with view mode, sort mode, and search (plain bar, no borders).
fn render_header(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let search_display = if state.search_active {
        format!("  Search: {}_", state.search_query)
    } else if !state.search_query.is_empty() {
        format!("  Search: {}", state.search_query)
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" View: {} ", state.view_mode.label()),
            Style::default().fg(theme::color::TEXT_PRIMARY),
        ),
        Span::styled("│", Style::default().fg(theme::color::SURFACE)),
        Span::styled(
            format!(" Sort: {} ", state.sort_mode.label()),
            Style::default().fg(theme::color::TEXT_PRIMARY),
        ),
        Span::styled(search_display, Style::default().fg(theme::color::ACTIVE)),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(theme::color::SURFACE));
    frame.render_widget(paragraph, area);
}

/// Render the branch list (borderless, old-TUI style inline indicators).
fn render_branch_list(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_branches();

    if filtered.is_empty() {
        let msg = if !state.branches.is_empty() {
            "No matching branches"
        } else {
            "No branches loaded"
        };
        let p = Paragraph::new(msg)
            .block(Block::default())
            .style(theme::style::muted_text());
        frame.render_widget(p, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, branch)| {
            let worktree_icon = if branch.worktree_path.is_some() {
                theme::icon::WORKTREE_ACTIVE
            } else {
                theme::icon::WORKTREE_INACTIVE
            };
            let head_indicator = if branch.is_head {
                theme::icon::HEAD_INDICATOR
            } else {
                ""
            };
            // Branch Cleanup gutter glyphs (FR-018c/d):
            //   `●` (cyan)   — selected for cleanup
            //   `✔` (green)  — cleanable now (merged into main or develop)
            //   spinner      — merge detection still running
            //   `·` (gray)   — local feature branch with unmerged commits
            //   `–` (dim)    — protected / current HEAD / active session
            //   ` ` (blank)  — remote-tracking branch (never a candidate)
            let spinner_glyph = state.merge_spinner_glyph();
            let (gutter_glyph, gutter_color) = if !branch.is_local {
                (" ", theme::color::TEXT_DISABLED)
            } else if state.is_cleanup_selected(&branch.name) {
                ("\u{25CF}", theme::color::ACTIVE)
            } else if !state.is_cleanable_candidate(&branch.name) {
                ("\u{2013}", theme::color::TEXT_DISABLED)
            } else {
                match state.merge_state(&branch.name) {
                    MergeState::Computing => (spinner_glyph, theme::color::ACTIVE),
                    MergeState::Cleanable(_) => ("\u{2714}", theme::color::SUCCESS),
                    MergeState::NotMerged => ("\u{00B7}", theme::color::TEXT_SECONDARY),
                }
            };

            let mut spans = vec![
                super::selection_prefix(idx == state.selected),
                Span::styled(gutter_glyph, Style::default().fg(gutter_color)),
                Span::raw(" "),
                Span::styled(
                    &branch.name,
                    Style::default().fg(theme::color::TEXT_PRIMARY),
                ),
                Span::raw(" "),
                Span::styled(worktree_icon, Style::default().fg(theme::color::FOCUS)),
                Span::styled(head_indicator, Style::default().fg(theme::color::SUCCESS)),
            ];
            let left_width = spans_width(&spans);
            let available_summary_width = area.width as usize;
            if let Some(summary_spans) =
                state
                    .live_session_summaries
                    .get(&branch.name)
                    .and_then(|summary| {
                        build_branch_live_summary(
                            summary,
                            state.session_animation_tick,
                            available_summary_width.saturating_sub(left_width.saturating_add(1)),
                        )
                    })
            {
                let summary_width = spans_width(&summary_spans);
                let gap = available_summary_width
                    .saturating_sub(left_width.saturating_add(summary_width))
                    .max(1);
                spans.push(Span::raw(" ".repeat(gap)));
                spans.extend(summary_spans);
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default());
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn build_branch_live_summary(
    summary: &BranchLiveSessionSummary,
    animation_tick: usize,
    available_width: usize,
) -> Option<Vec<Span<'static>>> {
    if available_width == 0 {
        return None;
    }

    let spinner = running_spinner_frame(animation_tick);
    let visible_indicators = summary.indicators.iter().take(available_width);
    let spans: Vec<Span<'static>> = visible_indicators
        .map(|indicator| {
            Span::styled(
                spinner.to_string(),
                Style::default().fg(branch_live_indicator_color(indicator.color)),
            )
        })
        .collect();

    if spans.is_empty() {
        None
    } else {
        Some(spans)
    }
}

fn running_spinner_frame(animation_tick: usize) -> char {
    const FRAMES: [char; 4] = ['◐', '◓', '◑', '◒'];
    FRAMES[animation_tick % FRAMES.len()]
}

fn branch_live_indicator_color(color: crate::model::AgentColor) -> Color {
    match color {
        crate::model::AgentColor::Green => Color::Green,
        crate::model::AgentColor::Blue => Color::Blue,
        crate::model::AgentColor::Cyan => Color::Cyan,
        crate::model::AgentColor::Yellow => Color::Yellow,
        crate::model::AgentColor::Magenta => Color::Magenta,
        crate::model::AgentColor::Gray => Color::Gray,
    }
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

/// Overview section: branch name, HEAD indicator, category, worktree path.
fn render_detail_overview(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let mut lines = Vec::new();

    match state.selected_branch() {
        Some(branch) => {
            let head = if branch.is_head { " (HEAD)" } else { "" };
            let locality = if branch.is_local { "Local" } else { "Remote" };
            lines.push(format!(" Branch: {}{}", branch.name, head));
            lines.push(format!(" Category: {}", branch.category.label()));
            lines.push(format!(" Type: {}", locality));
            if let Some(worktree) = branch.worktree_path.as_ref() {
                lines.push(format!(" Worktree: {}", worktree.display()));
            }
        }
        None => lines.push(" No branch selected".to_string()),
    }

    lines.push(String::new());
    lines.push(" Docker status".to_string());
    if state.selected_detail_loading() {
        lines.push(" Loading branch details...".to_string());
    } else if state.docker_containers.is_empty() {
        lines.push(" No containers found".to_string());
    } else if let Some(container) = state.selected_docker_container() {
        lines.push(format!(" Selected: {}", container.name));
        lines.push(format!(
            " Status: {}",
            docker_status_label(container.status)
        ));
        lines.push(format!(" Ports: {}", docker_ports_label(&container.ports)));
        lines.push(format!(
            " Controls: {}",
            docker_controls_hint(container.status)
        ));
    }

    let paragraph =
        Paragraph::new(lines.join("\n")).style(Style::default().fg(theme::color::TEXT_PRIMARY));
    frame.render_widget(paragraph, area);
}

fn docker_status_label(status: gwt_docker::ContainerStatus) -> &'static str {
    match status {
        gwt_docker::ContainerStatus::Created => "Created",
        gwt_docker::ContainerStatus::Running => "Running",
        gwt_docker::ContainerStatus::Paused => "Paused",
        gwt_docker::ContainerStatus::Stopped => "Stopped",
        gwt_docker::ContainerStatus::Exited => "Exited",
    }
}

fn docker_ports_label(ports: &str) -> &str {
    if ports.is_empty() {
        "No published ports"
    } else {
        ports
    }
}

fn docker_controls_hint(status: gwt_docker::ContainerStatus) -> &'static str {
    match status {
        gwt_docker::ContainerStatus::Running => "Up/Down select  T stop  R restart",
        gwt_docker::ContainerStatus::Paused => "Up/Down select  S start  T stop  R restart",
        gwt_docker::ContainerStatus::Created
        | gwt_docker::ContainerStatus::Stopped
        | gwt_docker::ContainerStatus::Exited => "Up/Down select  S start  R restart",
    }
}

/// SPECs section: list loaded from the worktree.
fn render_detail_specs(state: &BranchesState, frame: &mut Frame, area: Rect) {
    if state.selected_branch().is_none() {
        let paragraph = Paragraph::new(" No branch selected").style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    if state.detail_specs.is_empty() {
        if state.selected_detail_loading() {
            let paragraph = Paragraph::new(" Loading branch details...")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
            return;
        }
        let has_worktree = state
            .selected_branch()
            .and_then(|b| b.worktree_path.as_ref())
            .is_some();
        let msg = if has_worktree {
            " No SPECs found in worktree"
        } else {
            " No worktree (no SPECs available)"
        };
        let paragraph = Paragraph::new(msg).style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .detail_specs
        .iter()
        .map(|spec| {
            let line = Line::from(vec![
                Span::styled(format!(" {} ", spec.id), theme::style::header()),
                Span::styled(&spec.title, Style::default().fg(theme::color::TEXT_PRIMARY)),
                Span::styled(
                    format!("  [{}]", spec.status),
                    Style::default().fg(theme::color::ACTIVE),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
}

/// Git Status section: files and recent commits from the worktree.
fn render_detail_git_status(state: &BranchesState, frame: &mut Frame, area: Rect) {
    if state.selected_branch().is_none() {
        let paragraph = Paragraph::new(" No branch selected").style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    if state.selected_detail_loading() {
        let paragraph = Paragraph::new(" Loading branch details...")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let has_worktree = state
        .selected_branch()
        .and_then(|b| b.worktree_path.as_ref())
        .is_some();

    if !has_worktree {
        let paragraph = Paragraph::new(" No worktree (no git status available)")
            .style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Files section
    if state.detail_files.is_empty() {
        lines.push(Line::from(Span::styled(
            " Working tree clean",
            Style::default().fg(theme::color::SUCCESS),
        )));
    } else {
        lines.push(theme::section_divider(
            &format!("Changed files ({})", state.detail_files.len()),
            area.width,
        ));
        for file in &state.detail_files {
            let color = if file.starts_with("[S]") {
                theme::color::SUCCESS
            } else if file.starts_with("[?]") {
                theme::color::ERROR
            } else {
                theme::color::ACTIVE
            };
            lines.push(Line::from(Span::styled(
                format!("  {file}"),
                Style::default().fg(color),
            )));
        }
    }

    // Commits section
    if !state.detail_commits.is_empty() {
        lines.push(Line::from(""));
        lines.push(theme::section_divider("Recent commits", area.width));
        for commit in &state.detail_commits {
            lines.push(Line::from(Span::styled(
                format!("  {commit}"),
                Style::default().fg(theme::color::TEXT_PRIMARY),
            )));
        }
    }

    let paragraph = Paragraph::new(lines).style(Style::default().fg(theme::color::TEXT_PRIMARY));
    frame.render_widget(paragraph, area);
}

/// Sessions section: shows branch-scoped active session summaries.
fn render_detail_sessions(
    frame: &mut Frame,
    area: Rect,
    sessions: &[DetailSessionSummary],
    selected_session: usize,
) {
    if sessions.is_empty() {
        let paragraph = Paragraph::new(" No active sessions").style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    let mut lines = Vec::new();
    let selected_session = selected_session.min(sessions.len().saturating_sub(1));
    for (index, session) in sessions.iter().enumerate() {
        let selected_marker = if index == selected_session {
            theme::icon::LEFT_ACCENT
        } else {
            " "
        };
        let marker = if session.active { "●" } else { " " };
        let kind_style = match session.kind {
            "Agent" => Style::default().fg(theme::color::FOCUS),
            "Shell" => Style::default().fg(theme::color::SUCCESS),
            _ => Style::default().fg(theme::color::TEXT_PRIMARY),
        };
        let name_style = if index == selected_session || session.active {
            theme::style::active_item()
        } else {
            Style::default().fg(theme::color::TEXT_PRIMARY)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {selected_marker} {marker} "),
                Style::default().fg(theme::color::ACTIVE),
            ),
            Span::styled(session.kind, kind_style),
            Span::raw("  "),
            Span::styled(&session.name, name_style),
        ]));
        if let Some(detail) = session.detail.as_ref() {
            lines.push(Line::from(Span::styled(
                format!("   {detail}"),
                Style::default().fg(theme::color::SURFACE),
            )));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn sample_branches() -> Vec<BranchItem> {
        vec![
            BranchItem {
                name: "main".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Main,
                worktree_path: None,
            },
            BranchItem {
                name: "develop".to_string(),
                is_head: true,
                is_local: true,
                category: BranchCategory::Develop,
                worktree_path: None,
            },
            BranchItem {
                name: "feature/login".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Feature,
                worktree_path: None,
            },
            BranchItem {
                name: "origin/feature/api".to_string(),
                is_head: false,
                is_local: false,
                category: BranchCategory::Feature,
                worktree_path: None,
            },
            BranchItem {
                name: "hotfix/crash".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Other,
                worktree_path: None,
            },
        ]
    }

    fn sample_branches_with_early_remote() -> Vec<BranchItem> {
        vec![
            BranchItem {
                name: "origin/aaa-remote".to_string(),
                is_head: false,
                is_local: false,
                category: BranchCategory::Feature,
                worktree_path: None,
            },
            BranchItem {
                name: "zeta-local".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Other,
                worktree_path: None,
            },
            BranchItem {
                name: "yellow-local".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Other,
                worktree_path: None,
            },
            BranchItem {
                name: "origin/zzz-remote".to_string(),
                is_head: false,
                is_local: false,
                category: BranchCategory::Feature,
                worktree_path: None,
            },
        ]
    }

    fn sample_containers() -> Vec<gwt_docker::ContainerInfo> {
        vec![
            gwt_docker::ContainerInfo {
                id: "abc123".to_string(),
                name: "web".to_string(),
                status: gwt_docker::ContainerStatus::Running,
                image: "nginx:latest".to_string(),
                ports: "0.0.0.0:8080->80/tcp".to_string(),
            },
            gwt_docker::ContainerInfo {
                id: "def456".to_string(),
                name: "db".to_string(),
                status: gwt_docker::ContainerStatus::Stopped,
                image: "postgres:16".to_string(),
                ports: String::new(),
            },
        ]
    }

    fn buffer_to_lines(buf: &ratatui::buffer::Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn default_state() {
        let state = BranchesState::default();
        assert!(state.branches.is_empty());
        assert_eq!(state.selected, 0);
        assert_eq!(state.sort_mode, SortMode::Default);
        assert_eq!(state.view_mode, ViewMode::Local);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn move_down_stops_at_last_row() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.view_mode = ViewMode::All;
        assert_eq!(state.selected, 0);

        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        // Move to last
        for _ in 0..10 {
            update(&mut state, BranchesMessage::MoveDown);
        }
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn move_up_stops_at_first_row() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        assert_eq!(state.selected, 0);

        update(&mut state, BranchesMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, BranchesMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn select_returns_selected_branch() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.selected = 2;
        let branch = state.selected_branch().unwrap();
        assert_eq!(branch.name, "feature/login");
    }

    #[test]
    fn toggle_sort_cycles() {
        let mut state = BranchesState::default();
        assert_eq!(state.sort_mode, SortMode::Default);

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.sort_mode, SortMode::Name);

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.sort_mode, SortMode::Date);

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.sort_mode, SortMode::Default);
    }

    #[test]
    fn toggle_view_cycles() {
        let mut state = BranchesState::default();
        assert_eq!(state.view_mode, ViewMode::Local);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Remote);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::All);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Local);
    }

    #[test]
    fn search_start_activates() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::SearchStart);
        assert!(state.search_active);
    }

    #[test]
    fn search_input_appends() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::SearchStart);
        update(&mut state, BranchesMessage::SearchInput('f'));
        update(&mut state, BranchesMessage::SearchInput('e'));
        assert_eq!(state.search_query, "fe");
    }

    #[test]
    fn search_input_ignored_when_inactive() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::SearchInput('x'));
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn search_clear_resets() {
        let mut state = BranchesState::default();
        state.search_active = true;
        state.search_query = "test".to_string();

        update(&mut state, BranchesMessage::SearchClear);
        assert!(!state.search_active);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn set_branches_populates() {
        let mut state = BranchesState::default();
        state.selected = 99;
        update(&mut state, BranchesMessage::SetBranches(sample_branches()));
        assert_eq!(state.branches.len(), 5);
        assert_eq!(state.selected, 3); // clamped to last visible local row
    }

    #[test]
    fn set_branches_evicts_cached_detail_when_worktree_changes() {
        let mut state = BranchesState::default();
        let branch = BranchItem {
            name: "feature/api".to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/worktree-a")),
        };
        state.branches = vec![branch.clone()];
        state.detail_cache.insert(
            branch.name.clone(),
            BranchDetailData {
                files: vec!["stale.txt".to_string()],
                ..Default::default()
            },
        );

        let mut updated_branch = branch.clone();
        updated_branch.worktree_path = Some(PathBuf::from("/tmp/worktree-b"));
        update(
            &mut state,
            BranchesMessage::SetBranches(vec![updated_branch.clone()]),
        );

        assert!(!state.detail_cache.contains_key(&updated_branch.name));
        assert!(state.detail_files.is_empty());
    }

    #[test]
    fn toggle_sort_and_view_reset_detail_session_selection() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_session_selected = 3;

        update(&mut state, BranchesMessage::ToggleSort);
        assert_eq!(state.detail_session_selected, 0);

        state.detail_session_selected = 2;
        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.detail_session_selected, 0);
    }

    #[test]
    fn filtered_branches_respects_view_mode() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();

        state.view_mode = ViewMode::Local;
        assert_eq!(state.filtered_branches().len(), 4);

        state.view_mode = ViewMode::Remote;
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "origin/feature/api");
    }

    #[test]
    fn filtered_branches_respects_search() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.view_mode = ViewMode::All;
        state.search_query = "feature".to_string();

        let filtered = state.filtered_branches();
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn categorize_branch_works() {
        assert_eq!(categorize_branch("main"), BranchCategory::Main);
        assert_eq!(categorize_branch("master"), BranchCategory::Main);
        assert_eq!(categorize_branch("develop"), BranchCategory::Develop);
        assert_eq!(categorize_branch("feature/login"), BranchCategory::Feature);
        assert_eq!(
            categorize_branch("origin/feature/x"),
            BranchCategory::Feature
        );
        assert_eq!(categorize_branch("hotfix/crash"), BranchCategory::Other);
    }

    #[test]
    fn render_with_branches_does_not_panic() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // Header bar shows view/sort info (no bordered block title)
        assert!(text.contains("View:"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = BranchesState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
    }

    #[test]
    fn sort_name_returns_alphabetical_order_within_local_and_remote_groups() {
        let mut state = BranchesState::default();
        state.branches = sample_branches_with_early_remote();
        state.view_mode = ViewMode::All;
        state.sort_mode = SortMode::Name;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "yellow-local",
                "zeta-local",
                "origin/aaa-remote",
                "origin/zzz-remote",
            ]
        );
    }

    #[test]
    fn sort_name_keeps_local_branches_before_remote_branches() {
        let mut state = BranchesState::default();
        state.branches = sample_branches_with_early_remote();
        state.view_mode = ViewMode::All;
        state.sort_mode = SortMode::Name;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "yellow-local",
                "zeta-local",
                "origin/aaa-remote",
                "origin/zzz-remote",
            ]
        );
    }

    #[test]
    fn sort_default_keeps_local_branches_before_remote_branches() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.view_mode = ViewMode::All;
        state.sort_mode = SortMode::Default;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "main",
                "develop",
                "feature/login",
                "hotfix/crash",
                "origin/feature/api",
            ]
        );
    }

    #[test]
    fn sort_date_returns_alphabetical_fallback_within_local_and_remote_groups() {
        let mut state = BranchesState::default();
        state.branches = sample_branches_with_early_remote();
        state.view_mode = ViewMode::All;
        state.sort_mode = SortMode::Date;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "yellow-local",
                "zeta-local",
                "origin/aaa-remote",
                "origin/zzz-remote",
            ]
        );
    }

    #[test]
    fn sort_date_keeps_local_branches_before_remote_branches() {
        let mut state = BranchesState::default();
        state.branches = sample_branches_with_early_remote();
        state.view_mode = ViewMode::All;
        state.sort_mode = SortMode::Date;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "yellow-local",
                "zeta-local",
                "origin/aaa-remote",
                "origin/zzz-remote",
            ]
        );
    }

    #[test]
    fn search_then_navigate_selects_filtered_item() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.view_mode = ViewMode::All;

        // Search for "feature" — matches feature/login and origin/feature/api
        update(&mut state, BranchesMessage::SearchStart);
        update(&mut state, BranchesMessage::SearchInput('f'));
        update(&mut state, BranchesMessage::SearchInput('e'));
        update(&mut state, BranchesMessage::SearchInput('a'));
        update(&mut state, BranchesMessage::SearchInput('t'));

        let filtered = state.filtered_branches();
        assert_eq!(filtered.len(), 2);
        assert_eq!(state.selected, 0);

        // MoveDown should navigate within filtered list
        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        let branch = state.selected_branch().unwrap();
        assert_eq!(branch.name, "origin/feature/api");
    }

    #[test]
    fn select_returns_correct_branch_after_sort() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.sort_mode = SortMode::Name;

        // After sort by name: develop, feature/login, hotfix/crash, main, origin/feature/api
        state.selected = 3;
        let branch = state.selected_branch().unwrap();
        assert_eq!(branch.name, "main");
    }

    #[test]
    fn view_toggle_clamps_selected() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.view_mode = ViewMode::All;
        state.selected = 4; // last item (Other)

        // Switch to Remote — only 1 item
        update(&mut state, BranchesMessage::ToggleView);
        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Remote);
        assert_eq!(state.selected, 0); // clamped
    }

    // ---- Branch detail tests ----

    #[test]
    fn detail_section_defaults_to_zero() {
        let state = BranchesState::default();
        assert_eq!(state.detail_section, 0);
    }

    #[test]
    fn next_detail_section_cycles_through_all() {
        let mut state = BranchesState::default();
        for expected in 1..=3 {
            update(&mut state, BranchesMessage::NextDetailSection);
            assert_eq!(state.detail_section, expected);
        }
        // Wraps back to 0
        update(&mut state, BranchesMessage::NextDetailSection);
        assert_eq!(state.detail_section, 0);
    }

    #[test]
    fn prev_detail_section_wraps_from_zero() {
        let mut state = BranchesState::default();
        assert_eq!(state.detail_section, 0);
        update(&mut state, BranchesMessage::PrevDetailSection);
        assert_eq!(state.detail_section, 3);
    }

    #[test]
    fn prev_detail_section_decrements() {
        let mut state = BranchesState::default();
        state.detail_section = 3;
        update(&mut state, BranchesMessage::PrevDetailSection);
        assert_eq!(state.detail_section, 2);
    }

    #[test]
    fn launch_agent_sets_flag() {
        let mut state = BranchesState::default();
        assert!(!state.pending_launch_agent);
        update(&mut state, BranchesMessage::LaunchAgent);
        assert!(state.pending_launch_agent);
    }

    #[test]
    fn open_shell_sets_flag() {
        let mut state = BranchesState::default();
        assert!(!state.pending_open_shell);
        update(&mut state, BranchesMessage::OpenShell);
        assert!(state.pending_open_shell);
    }

    #[test]
    fn render_with_detail_split_does_not_panic() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        // Header bar shows view/sort info (no bordered block title)
        assert!(text.contains("View:"));
    }

    #[test]
    fn render_branch_list_uses_inline_indicators_without_headers_or_locality_badges() {
        let mut state = BranchesState::default();
        state.branches = vec![
            BranchItem {
                name: "main".to_string(),
                is_head: true,
                is_local: true,
                category: BranchCategory::Main,
                worktree_path: None,
            },
            BranchItem {
                name: "feature/worktree".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Feature,
                worktree_path: Some(PathBuf::from("/tmp/worktree")),
            },
        ];

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_list(&state, f, f.area());
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        let joined = lines.join("\n");

        assert!(!joined.contains("── Main ──"));
        assert!(!joined.contains("[L]"));
        assert!(!joined.contains("[R]"));
        assert!(joined.contains("main \u{25C7} \u{25B8}"));
        assert!(joined.contains("feature/worktree \u{25C6}"));
    }

    #[test]
    fn render_detail_overview_omits_redundant_inner_title() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 0; // Overview
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found_branch_info = false;
        let mut found_inner_title = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains(" Branch: main") {
                found_branch_info = true;
            }
            if line.contains("Overview") {
                found_inner_title = true;
            }
        }
        assert!(
            found_branch_info,
            "Detail panel should still contain branch overview body text"
        );
        assert!(
            !found_inner_title,
            "Detail panel body should not repeat the redundant inner 'Overview' title"
        );
    }

    #[test]
    fn render_detail_overview_shows_docker_status_area() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 0; // Overview
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        let mut found_docker_status = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("Docker status") {
                found_docker_status = true;
                break;
            }
        }

        assert!(
            found_docker_status,
            "Detail panel should contain a Docker status area"
        );
    }

    #[test]
    fn render_detail_overview_shows_no_containers_message() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 0;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines
                .iter()
                .any(|line| line.contains("No containers found")),
            "Detail panel should explain when there are no Docker containers"
        );
    }

    #[test]
    fn load_branch_detail_populates_docker_containers() {
        let tmp = tempfile::tempdir().expect("create temp worktree");
        let branch = BranchItem {
            name: "feature/docker".to_string(),
            is_head: true,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: Some(tmp.path().to_path_buf()),
        };

        let detail = load_branch_detail(&branch, &sample_containers());

        assert_eq!(detail.docker_containers.len(), 2);
        let container = &detail.docker_containers[0];
        assert_eq!(container.id, "abc123");
        assert_eq!(container.name, "web");
        assert_eq!(container.status, gwt_docker::ContainerStatus::Running);
        assert_eq!(container.ports, "0.0.0.0:8080->80/tcp");
    }

    #[test]
    fn docker_selection_and_lifecycle_intent_update_state() {
        let mut state = BranchesState::default();
        state.docker_containers = sample_containers();

        update(&mut state, BranchesMessage::DockerContainerDown);
        assert_eq!(state.docker_selected, 1);

        update(&mut state, BranchesMessage::DockerContainerUp);
        assert_eq!(state.docker_selected, 0);

        update(&mut state, BranchesMessage::DockerContainerRestart);
        assert_eq!(
            state.pending_docker_action,
            Some(PendingDockerAction {
                container_id: "abc123".to_string(),
                action: DockerLifecycleAction::Restart,
            })
        );
    }

    #[test]
    fn render_detail_overview_shows_selected_docker_container_details() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.docker_containers = sample_containers();
        state.docker_selected = 1;
        state.detail_section = 0;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines.iter().any(|line| line.contains("Selected: db")),
            "Detail panel should show the selected Docker container"
        );
        assert!(
            lines.iter().any(|line| line.contains("Status: Stopped")),
            "Detail panel should show Docker status"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Ports: No published ports")),
            "Detail panel should show Docker ports"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Controls: Up/Down select  S start  R restart")),
            "Detail panel should show Docker control hints"
        );
    }

    #[test]
    fn render_detail_sessions_shows_typed_session_rows_and_active_marker() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 3;
        let sessions = vec![
            DetailSessionSummary {
                kind: "Agent",
                name: "Codex".to_string(),
                detail: Some("gpt-5.3-codex · high".to_string()),
                active: true,
            },
            DetailSessionSummary {
                kind: "Shell",
                name: "Shell: develop".to_string(),
                detail: None,
                active: false,
            },
        ];

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_detail_content(&state, f, area, &sessions);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines.iter().any(|line| line.contains("● Agent  Codex")),
            "Sessions pane should show the active agent row"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Shell  Shell: develop")),
            "Sessions pane should show branch shell rows"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("gpt-5.3-codex · high")),
            "Sessions pane should show session detail metadata when available"
        );
    }

    #[test]
    fn render_detail_sessions_preserves_empty_state() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 3;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_detail_content(&state, f, area, &[]);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines.iter().any(|line| line.contains("No active sessions")),
            "Sessions pane should keep the empty-state fallback"
        );
    }

    #[test]
    fn render_detail_sessions_shows_selection_marker_for_current_row() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.detail_section = 3;
        state.detail_session_selected = 1;
        let sessions = vec![
            DetailSessionSummary {
                kind: "Agent",
                name: "Codex".to_string(),
                detail: None,
                active: false,
            },
            DetailSessionSummary {
                kind: "Shell",
                name: "Shell: develop".to_string(),
                detail: None,
                active: true,
            },
        ];

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_detail_content(&state, f, area, &sessions);
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines
                .iter()
                .any(|line| line.contains("\u{258E} \u{25CF} Shell  Shell: develop")),
            "Sessions pane should show the selected-row marker on the current row"
        );
    }

    #[test]
    fn render_branch_list_shows_running_summary_on_wide_rows() {
        let mut state = BranchesState::default();
        state.branches = vec![BranchItem {
            name: "feature/live".to_string(),
            is_head: true,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-live")),
        }];
        state.live_session_summaries.insert(
            "feature/live".to_string(),
            BranchLiveSessionSummary {
                indicators: vec![BranchLiveSessionIndicator {
                    status: gwt_agent::AgentStatus::Running,
                    color: crate::model::AgentColor::Blue,
                }],
            },
        );
        state.session_animation_tick = 0;

        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_list(&state, f, f.area());
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines
                .iter()
                .any(|line| line.contains("feature/live") && line.contains("◐") && !line.contains("run ")),
            "wide rows should keep the old-TUI left side and add a right-aligned spinner-only indicator"
        );
    }

    #[test]
    fn render_branch_list_keeps_waiting_sessions_visible_with_spinner_indicator() {
        let mut state = BranchesState::default();
        state.branches = vec![BranchItem {
            name: "feature/wait".to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-wait")),
        }];
        state.live_session_summaries.insert(
            "feature/wait".to_string(),
            BranchLiveSessionSummary {
                indicators: vec![BranchLiveSessionIndicator {
                    status: gwt_agent::AgentStatus::WaitingInput,
                    color: crate::model::AgentColor::Green,
                }],
            },
        );
        state.session_animation_tick = 0;

        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_list(&state, f, f.area());
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines.iter().any(|line| line.contains("feature/wait"))
                && lines.iter().any(|line| line.contains("◐"))
                && !lines.iter().any(|line| line.contains(" wait ")),
            "waiting rows should keep a spinner-only indicator instead of a textual wait label"
        );
    }

    #[test]
    fn render_branch_list_omits_live_summary_before_branch_label_on_narrow_rows() {
        let mut state = BranchesState::default();
        state.branches = vec![BranchItem {
            name: "feature/narrow".to_string(),
            is_head: true,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: Some(PathBuf::from("/tmp/wt-feature-narrow")),
        }];
        state.live_session_summaries.insert(
            "feature/narrow".to_string(),
            BranchLiveSessionSummary {
                indicators: vec![BranchLiveSessionIndicator {
                    status: gwt_agent::AgentStatus::Running,
                    color: crate::model::AgentColor::Blue,
                }],
            },
        );

        let backend = TestBackend::new(24, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_list(&state, f, f.area());
            })
            .unwrap();

        let lines = buffer_to_lines(terminal.backend().buffer());
        assert!(
            lines.iter().any(|line| line.contains("feature/narrow")),
            "narrow rows should preserve the branch label even if the spinner strip must disappear"
        );
    }

    #[test]
    fn detail_section_labels_are_correct() {
        // Detail section tab labels are now rendered by app.rs in the pane border.
        // Verify the labels returned by detail_section_labels().
        let labels = detail_section_labels();
        assert!(labels.contains(&"Overview"));
        assert!(labels.contains(&"SPECs"));
        assert!(labels.contains(&"Git"));
        assert!(labels.contains(&"Sessions"));
    }

    #[test]
    fn render_no_selected_branch_shows_placeholder() {
        let state = BranchesState::default(); // empty branches
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found_no_branch = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("No branch selected") {
                found_no_branch = true;
                break;
            }
        }
        assert!(
            found_no_branch,
            "Should show 'No branch selected' when empty"
        );
    }

    // ---------------- Branch Cleanup state (FR-018) ----------------

    fn feature(name: &str) -> BranchItem {
        BranchItem {
            name: name.to_string(),
            is_head: false,
            is_local: true,
            category: BranchCategory::Feature,
            worktree_path: None,
        }
    }

    fn cleanup_state_with(branches: Vec<BranchItem>) -> BranchesState {
        let mut state = BranchesState::default();
        state.branches = branches;
        state
    }

    #[test]
    fn merge_state_defaults_to_computing() {
        let state = BranchesState::default();
        assert_eq!(state.merge_state("feature/foo"), MergeState::Computing);
    }

    #[test]
    fn toggle_cleanup_selection_skips_computing_branches() {
        let mut state = cleanup_state_with(vec![feature("feature/foo")]);
        // Default state is Computing → not selectable.
        assert!(!state.toggle_cleanup_selection("feature/foo"));
        assert!(state.cleanup_selected.is_empty());
    }

    #[test]
    fn toggle_cleanup_selection_adds_then_removes_cleanable_branch() {
        let mut state = cleanup_state_with(vec![feature("feature/foo")]);
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );

        assert!(state.toggle_cleanup_selection("feature/foo"));
        assert!(state.is_cleanup_selected("feature/foo"));

        assert!(state.toggle_cleanup_selection("feature/foo"));
        assert!(!state.is_cleanup_selected("feature/foo"));
    }

    #[test]
    fn toggle_cleanup_selection_rejects_protected_branches() {
        let mut state = cleanup_state_with(vec![BranchItem {
            name: "main".to_string(),
            is_head: true,
            is_local: true,
            category: BranchCategory::Main,
            worktree_path: None,
        }]);
        state.set_merge_state("main", MergeState::Cleanable(gwt_git::MergeTarget::Main));
        assert!(!state.toggle_cleanup_selection("main"));
    }

    #[test]
    fn toggle_cleanup_selection_rejects_current_head_branch() {
        let mut state = cleanup_state_with(vec![feature("feature/foo")]);
        state.current_head_branch = Some("feature/foo".to_string());
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        assert!(!state.toggle_cleanup_selection("feature/foo"));
    }

    #[test]
    fn toggle_cleanup_selection_rejects_branch_with_active_session() {
        let mut state = cleanup_state_with(vec![feature("feature/foo")]);
        state
            .active_session_branches
            .insert("feature/foo".to_string());
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        assert!(!state.toggle_cleanup_selection("feature/foo"));
    }

    #[test]
    fn toggle_cleanup_selection_allows_branch_with_worktree() {
        // FR-018b (revised): feature branches that have their own worktree
        // are still cleanup candidates — removing the worktree is the whole
        // point of Branch Cleanup in a gwt workflow.
        let mut state = cleanup_state_with(vec![feature("feature/foo")]);
        state.checked_out_branches.insert("feature/foo".to_string());
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        assert!(state.toggle_cleanup_selection("feature/foo"));
    }

    #[test]
    fn select_all_visible_cleanable_only_picks_cleanable_rows() {
        let mut state = cleanup_state_with(vec![
            feature("feature/foo"),
            feature("feature/bar"),
            feature("feature/baz"),
            BranchItem {
                name: "main".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Main,
                worktree_path: None,
            },
        ]);
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        state.set_merge_state(
            "feature/bar",
            MergeState::Cleanable(gwt_git::MergeTarget::Develop),
        );
        state.set_merge_state("feature/baz", MergeState::NotMerged);
        // main stays Computing → also rejected via protected guard.

        let visible_owned: Vec<BranchItem> = state.branches.clone();
        let visible: Vec<&BranchItem> = visible_owned.iter().collect();
        state.select_all_visible_cleanable(&visible);

        assert!(state.is_cleanup_selected("feature/foo"));
        assert!(state.is_cleanup_selected("feature/bar"));
        assert!(!state.is_cleanup_selected("feature/baz"));
        assert!(!state.is_cleanup_selected("main"));
        assert_eq!(state.cleanup_selection_count(), 2);
    }

    #[test]
    fn cleanup_selection_persists_across_view_mode_and_sort_changes() {
        let mut state = cleanup_state_with(vec![feature("feature/foo")]);
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        assert!(state.toggle_cleanup_selection("feature/foo"));

        // Simulate the view-mode toggle and sort cycle paths.
        update(&mut state, BranchesMessage::ToggleView);
        update(&mut state, BranchesMessage::ToggleSort);
        update(&mut state, BranchesMessage::SearchStart);
        update(&mut state, BranchesMessage::SearchInput('f'));
        update(&mut state, BranchesMessage::SearchClear);

        assert!(state.is_cleanup_selected("feature/foo"));
    }

    #[test]
    fn clear_selection_after_cleanup_drops_all_selections() {
        let mut state = cleanup_state_with(vec![feature("feature/foo"), feature("feature/bar")]);
        state.set_merge_state(
            "feature/foo",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        state.set_merge_state(
            "feature/bar",
            MergeState::Cleanable(gwt_git::MergeTarget::Main),
        );
        state.toggle_cleanup_selection("feature/foo");
        state.toggle_cleanup_selection("feature/bar");
        assert_eq!(state.cleanup_selection_count(), 2);

        state.clear_selection_after_cleanup();
        assert_eq!(state.cleanup_selection_count(), 0);
    }

    #[test]
    fn cleanup_run_state_records_progress_and_finishes() {
        let mut run = CleanupRunState::new(3, false);
        assert_eq!(run.phase, CleanupRunPhase::Running);
        assert_eq!(run.total, 3);
        assert_eq!(run.processed, 0);

        run.current = Some("feature/foo".to_string());
        run.record_result(CleanupResultRow {
            branch: "feature/foo".to_string(),
            success: true,
            message: None,
        });
        run.record_result(CleanupResultRow {
            branch: "feature/bar".to_string(),
            success: false,
            message: Some("worktree busy".to_string()),
        });
        run.record_result(CleanupResultRow {
            branch: "feature/baz".to_string(),
            success: true,
            message: None,
        });
        run.finish();

        assert_eq!(run.processed, 3);
        assert_eq!(run.succeeded(), 2);
        assert_eq!(run.failed(), 1);
        assert_eq!(run.phase, CleanupRunPhase::Done);
        assert!(run.current.is_none());
    }
}
