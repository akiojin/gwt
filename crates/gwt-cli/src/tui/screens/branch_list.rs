//! Branch List Screen - TypeScript版完全互換

#![allow(dead_code)]

use gwt_core::config::AgentStatus;
use gwt_core::git::{Branch, BranchMeta, BranchSummary, DivergenceStatus, Repository};
use gwt_core::tmux::AgentPane;
use gwt_core::worktree::Worktree;
use ratatui::{prelude::*, widgets::*};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

/// Get terminal color for coding agent (SPEC-3b0ed29b FR-024~FR-027)
fn get_agent_color(tool_id: Option<&str>) -> Color {
    match tool_id {
        Some(id) => match id.to_lowercase().as_str() {
            "claude-code" | "claude" => Color::Yellow,
            "codex-cli" | "codex" => Color::Cyan,
            "gemini-cli" | "gemini" => Color::Magenta,
            "opencode" | "open-code" => Color::Green,
            _ if id.to_lowercase().contains("claude") => Color::Yellow,
            _ if id.to_lowercase().contains("codex") => Color::Cyan,
            _ if id.to_lowercase().contains("gemini") => Color::Magenta,
            _ if id.to_lowercase().contains("opencode") => Color::Green,
            _ => Color::White,
        },
        None => Color::Gray,
    }
}

/// Get display name for agent (capitalize first letter)
fn get_agent_display_name(agent_name: &str) -> String {
    crate::tui::normalize_agent_label(agent_name)
}

/// Branch name type for sorting priority (SPEC-d2f4762a FR-003a)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BranchNameType {
    Main,
    Develop,
    Feature,
    Bugfix,
    Hotfix,
    Release,
    Other,
}

/// Get branch name type for sorting
fn get_branch_name_type(name: &str) -> BranchNameType {
    let lower = name.to_lowercase();
    // Strip remote prefix for comparison
    let name_part = lower.split('/').next_back().unwrap_or(&lower);

    if name_part == "main" || name_part == "master" {
        BranchNameType::Main
    } else if name_part == "develop" || name_part == "dev" {
        BranchNameType::Develop
    } else if lower.contains("feature/") {
        BranchNameType::Feature
    } else if lower.contains("bugfix/") || lower.contains("bug/") {
        BranchNameType::Bugfix
    } else if lower.contains("hotfix/") {
        BranchNameType::Hotfix
    } else if lower.contains("release/") {
        BranchNameType::Release
    } else {
        BranchNameType::Other
    }
}

/// View mode for branch list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    All,
    Local,
    Remote,
}

impl ViewMode {
    pub fn label(&self) -> &'static str {
        match self {
            ViewMode::All => "All",
            ViewMode::Local => "Local",
            ViewMode::Remote => "Remote",
        }
    }

    pub fn cycle(&self) -> Self {
        match self {
            ViewMode::All => ViewMode::Local,
            ViewMode::Local => ViewMode::Remote,
            ViewMode::Remote => ViewMode::All,
        }
    }
}

/// Branch type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BranchType {
    #[default]
    Local,
    Remote,
}

/// Safety status for cleanup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SafetyStatus {
    #[default]
    Unknown,
    Pending,
    Safe,
    Uncommitted,
    Unpushed,
    Unmerged,
    Unsafe,
}

impl SafetyStatus {
    pub fn is_unsafe(self) -> bool {
        !matches!(self, SafetyStatus::Safe)
    }
}

/// Statistics for branch list
#[derive(Debug, Clone, Default)]
pub struct Statistics {
    pub local_count: usize,
    pub remote_count: usize,
    pub worktree_count: usize,
    pub changes_count: usize,
}

/// Branch item with full information
#[derive(Debug, Clone)]
pub struct BranchItem {
    pub name: String,
    pub branch_type: BranchType,
    pub is_current: bool,
    pub has_worktree: bool,
    pub worktree_path: Option<String>,
    pub worktree_status: WorktreeStatus,
    pub has_changes: bool,
    pub has_unpushed: bool,
    pub divergence: DivergenceStatus,
    pub has_remote_counterpart: bool,
    pub remote_name: Option<String>,
    pub safe_to_cleanup: Option<bool>,
    pub safety_status: SafetyStatus,
    pub is_unmerged: bool,
    pub last_commit_timestamp: Option<i64>,
    pub last_tool_usage: Option<String>,
    pub is_selected: bool,
    /// PR title for search (FR-016)
    pub pr_title: Option<String>,
}

/// Worktree status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorktreeStatus {
    #[default]
    None,
    Active,
    Inaccessible,
}

impl BranchItem {
    pub fn from_branch(branch: &Branch, worktrees: &[Worktree]) -> Self {
        let worktree = worktrees.iter().find(|wt| {
            wt.branch
                .as_ref()
                .map(|b| b == &branch.name)
                .unwrap_or(false)
        });

        let worktree_status = if let Some(wt) = worktree {
            if wt.path.exists() {
                WorktreeStatus::Active
            } else {
                WorktreeStatus::Inaccessible
            }
        } else {
            WorktreeStatus::None
        };

        let branch_type = if branch.name.starts_with("remotes/") {
            BranchType::Remote
        } else {
            BranchType::Local
        };

        let mut item = Self {
            name: branch.name.clone(),
            branch_type,
            is_current: branch.is_current,
            has_worktree: worktree.is_some(),
            worktree_path: worktree.map(|wt| wt.path.display().to_string()),
            worktree_status,
            has_changes: worktree.map(|wt| wt.has_changes).unwrap_or(false),
            has_unpushed: worktree.map(|wt| wt.has_unpushed).unwrap_or(false),
            divergence: DivergenceStatus::UpToDate,
            has_remote_counterpart: branch.has_remote,
            remote_name: if branch.name.starts_with("remotes/") {
                Some(branch.name.clone())
            } else {
                None
            },
            safe_to_cleanup: None,
            safety_status: SafetyStatus::Unknown,
            is_unmerged: false,
            // FR-041: Set commit timestamp from git
            last_commit_timestamp: branch.commit_timestamp,
            last_tool_usage: None,
            is_selected: false,
            pr_title: None, // FR-016: Will be populated from PrCache
        };
        item.update_safety_status();
        item
    }

    pub fn from_branch_minimal(branch: &Branch, worktrees: &[Worktree]) -> Self {
        let worktree = worktrees.iter().find(|wt| {
            wt.branch
                .as_ref()
                .map(|b| b == &branch.name)
                .unwrap_or(false)
        });

        let branch_type = if branch.name.starts_with("remotes/") {
            BranchType::Remote
        } else {
            BranchType::Local
        };

        let mut item = Self {
            name: branch.name.clone(),
            branch_type,
            is_current: branch.is_current,
            has_worktree: worktree.is_some(),
            worktree_path: worktree.map(|wt| wt.path.display().to_string()),
            worktree_status: if worktree.is_some() {
                WorktreeStatus::Active
            } else {
                WorktreeStatus::None
            },
            has_changes: false,
            has_unpushed: false,
            divergence: DivergenceStatus::UpToDate,
            has_remote_counterpart: branch.has_remote,
            remote_name: if branch.name.starts_with("remotes/") {
                Some(branch.name.clone())
            } else {
                None
            },
            safe_to_cleanup: None,
            safety_status: SafetyStatus::Unknown,
            is_unmerged: false,
            last_commit_timestamp: branch.commit_timestamp,
            last_tool_usage: None,
            is_selected: false,
            pr_title: None,
        };
        item.update_safety_status();
        item
    }

    pub fn update_safety_status(&mut self) {
        self.safety_status = if self.branch_type == BranchType::Remote {
            SafetyStatus::Unknown
        } else if self.has_changes {
            SafetyStatus::Uncommitted
        } else if self.has_unpushed {
            SafetyStatus::Unpushed
        } else if self.is_unmerged {
            SafetyStatus::Unmerged
        } else if self.safe_to_cleanup == Some(true) {
            SafetyStatus::Safe
        } else if self.safe_to_cleanup.is_none() {
            SafetyStatus::Pending
        } else {
            SafetyStatus::Unsafe
        };
    }

    pub fn is_unsafe(&self) -> bool {
        self.safety_status.is_unsafe()
    }

    /// Get safety icon and color
    /// FR-031b: If spinner_frame is provided and safety check is pending, show spinner
    pub fn safety_icon(&self, spinner_frame: Option<usize>) -> (String, Color) {
        if self.branch_type == BranchType::Remote {
            // FR-031c: Remote branches show empty safety icon
            return (" ".to_string(), Color::Reset);
        }
        match self.safety_status {
            SafetyStatus::Uncommitted => ("!".to_string(), Color::Red),
            SafetyStatus::Unpushed => ("^".to_string(), Color::Yellow),
            SafetyStatus::Unmerged => ("*".to_string(), Color::Yellow),
            SafetyStatus::Safe => ("o".to_string(), Color::Green),
            SafetyStatus::Pending => {
                // FR-031b: Safety check pending - show spinner if frame provided
                if let Some(frame) = spinner_frame {
                    let spinner_char = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
                    (spinner_char.to_string(), Color::Yellow)
                } else {
                    ("!".to_string(), Color::Red)
                }
            }
            SafetyStatus::Unknown | SafetyStatus::Unsafe => ("!".to_string(), Color::Red),
        }
    }

    /// Get worktree icon and color
    pub fn worktree_icon(&self) -> (&'static str, Color) {
        match self.worktree_status {
            WorktreeStatus::Active => ("w", Color::LightGreen),
            WorktreeStatus::Inaccessible => ("x", Color::Red),
            WorktreeStatus::None => (".", Color::DarkGray),
        }
    }
}

/// Spinner animation frames (for loading indicators)
const SPINNER_FRAMES: &[char] = &['|', '/', '-', '\\'];

/// Active agent spinner (star twinkle)
const ACTIVE_SPINNER_FRAMES: &[char] = &['✶', '✸', '✹', '✺', '✹', '✷'];

/// Background agent spinner (BLACK_CIRCLE)
const BG_SPINNER_FRAMES: &[char] = &['◑', '◒', '◐', '◓'];

/// Branch list state
#[derive(Debug)]
pub struct BranchListState {
    pub branches: Vec<BranchItem>,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub offset: usize,
    pub filter: String,
    pub filter_mode: bool,
    pub view_mode: ViewMode,
    pub selected_branches: HashSet<String>,
    pub stats: Statistics,
    pub is_loading: bool,
    pub loading_started: Option<Instant>,
    pub error: Option<String>,
    pub version: Option<String>,
    pub working_directory: Option<String>,
    pub active_profile: Option<String>,
    pub spinner_frame: usize,
    pub filter_cache_version: u64,
    pub status_progress_total: usize,
    pub status_progress_done: usize,
    pub status_progress_active: bool,
    /// Viewport height for scroll calculations (updated by renderer)
    pub visible_height: usize,
    /// Running agents mapped by branch name (for agent info display)
    pub running_agents: HashMap<String, AgentPane>,
    /// Branch summary data for the selected branch (SPEC-4b893dae)
    pub branch_summary: Option<BranchSummary>,
}

impl Default for BranchListState {
    fn default() -> Self {
        Self {
            branches: Vec::new(),
            filtered_indices: Vec::new(),
            selected: 0,
            offset: 0,
            filter: String::new(),
            filter_mode: false,
            view_mode: ViewMode::default(),
            selected_branches: HashSet::new(),
            stats: Statistics::default(),
            is_loading: false,
            loading_started: None,
            error: None,
            version: None,
            working_directory: None,
            active_profile: None,
            spinner_frame: 0,
            filter_cache_version: 0,
            status_progress_total: 0,
            status_progress_done: 0,
            status_progress_active: false,
            visible_height: 15, // Default fallback (previously hardcoded)
            running_agents: HashMap::new(),
            branch_summary: None,
        }
    }
}

impl BranchListState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_branches(mut self, branches: Vec<BranchItem>) -> Self {
        // Calculate statistics
        self.stats = Statistics {
            local_count: branches
                .iter()
                .filter(|b| b.branch_type == BranchType::Local)
                .count(),
            remote_count: branches
                .iter()
                .filter(|b| b.branch_type == BranchType::Remote || b.has_remote_counterpart)
                .count(),
            worktree_count: branches.iter().filter(|b| b.has_worktree).count(),
            changes_count: branches.iter().filter(|b| b.has_changes).count(),
        };
        self.branches = branches;
        self.rebuild_filtered_cache();
        self
    }

    fn rebuild_filtered_cache(&mut self) {
        let mut result: Vec<usize> = self.branches.iter().enumerate().map(|(i, _)| i).collect();

        result.retain(|&index| {
            let branch = &self.branches[index];
            match self.view_mode {
                ViewMode::All => true,
                ViewMode::Local => branch.branch_type == BranchType::Local,
                ViewMode::Remote => {
                    branch.branch_type == BranchType::Remote || branch.has_remote_counterpart
                }
            }
        });

        if !self.filter.is_empty() {
            let filter_lower = self.filter.to_lowercase();
            result.retain(|&index| {
                let branch = &self.branches[index];
                if branch.name.to_lowercase().contains(&filter_lower) {
                    return true;
                }
                if let Some(ref pr_title) = branch.pr_title {
                    return pr_title.to_lowercase().contains(&filter_lower);
                }
                false
            });
        }

        let has_main = result
            .iter()
            .any(|&index| get_branch_name_type(&self.branches[index].name) == BranchNameType::Main);

        result.sort_by(|&a_index, &b_index| {
            let a = &self.branches[a_index];
            let b = &self.branches[b_index];

            if a.is_current && !b.is_current {
                return std::cmp::Ordering::Less;
            }
            if !a.is_current && b.is_current {
                return std::cmp::Ordering::Greater;
            }

            let a_type = get_branch_name_type(&a.name);
            let b_type = get_branch_name_type(&b.name);
            if a_type == BranchNameType::Main && b_type != BranchNameType::Main {
                return std::cmp::Ordering::Less;
            }
            if a_type != BranchNameType::Main && b_type == BranchNameType::Main {
                return std::cmp::Ordering::Greater;
            }

            if has_main {
                if a_type == BranchNameType::Develop && b_type != BranchNameType::Develop {
                    return std::cmp::Ordering::Less;
                }
                if a_type != BranchNameType::Develop && b_type == BranchNameType::Develop {
                    return std::cmp::Ordering::Greater;
                }
            }

            if a.has_worktree && !b.has_worktree {
                return std::cmp::Ordering::Less;
            }
            if !a.has_worktree && b.has_worktree {
                return std::cmp::Ordering::Greater;
            }

            if let (Some(a_time), Some(b_time)) = (a.last_commit_timestamp, b.last_commit_timestamp)
            {
                if a_time != b_time {
                    return b_time.cmp(&a_time);
                }
            } else if a.last_commit_timestamp.is_some() {
                return std::cmp::Ordering::Less;
            } else if b.last_commit_timestamp.is_some() {
                return std::cmp::Ordering::Greater;
            }

            if a.branch_type == BranchType::Local && b.branch_type == BranchType::Remote {
                return std::cmp::Ordering::Less;
            }
            if a.branch_type == BranchType::Remote && b.branch_type == BranchType::Local {
                return std::cmp::Ordering::Greater;
            }

            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        });

        self.filtered_indices = result;
        self.filter_cache_version = self.filter_cache_version.wrapping_add(1);
        if !self.filtered_indices.is_empty() {
            self.selected = self.selected.min(self.filtered_indices.len() - 1);
        } else {
            self.selected = 0;
            self.offset = 0;
        }
        self.ensure_visible();
    }

    pub fn filtered_len(&self) -> usize {
        self.filtered_indices.len()
    }

    pub fn filtered_branch_at(&self, index: usize) -> Option<&BranchItem> {
        self.filtered_indices
            .get(index)
            .and_then(|&idx| self.branches.get(idx))
    }

    pub fn visible_filtered_indices(&self, visible_height: usize) -> &[usize] {
        let start = self.offset.min(self.filtered_indices.len());
        let end = (start + visible_height).min(self.filtered_indices.len());
        &self.filtered_indices[start..end]
    }

    /// Get filtered branches based on view mode and filter
    /// Sorted according to SPEC-d2f4762a FR-003a:
    /// 1. Current branch (highest priority)
    /// 2. main branch
    /// 3. develop branch (only if main exists)
    /// 4. Branches with worktree
    /// 5. Latest activity timestamp (descending)
    /// 6. Local branches (over remote)
    /// 7. Alphabetical order
    pub fn filtered_branches(&self) -> Vec<&BranchItem> {
        self.filtered_indices
            .iter()
            .filter_map(|&index| self.branches.get(index))
            .collect()
    }

    /// Cycle view mode
    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.cycle();
        self.selected = 0;
        self.offset = 0;
        self.rebuild_filtered_cache();
    }

    pub fn set_view_mode(&mut self, view_mode: ViewMode) {
        self.view_mode = view_mode;
        self.selected = 0;
        self.offset = 0;
        self.rebuild_filtered_cache();
    }

    /// Toggle filter mode
    pub fn toggle_filter_mode(&mut self) {
        self.filter_mode = !self.filter_mode;
    }

    /// Enter filter mode
    pub fn enter_filter_mode(&mut self) {
        self.filter_mode = true;
    }

    /// Exit filter mode
    pub fn exit_filter_mode(&mut self) {
        self.filter_mode = false;
    }

    /// Clear filter
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected = 0;
        self.offset = 0;
        self.rebuild_filtered_cache();
    }

    /// Add char to filter
    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
        self.offset = 0;
        self.rebuild_filtered_cache();
    }

    /// Remove char from filter
    pub fn filter_pop(&mut self) {
        self.filter.pop();
        self.rebuild_filtered_cache();
    }

    /// Toggle selection for current branch
    pub fn toggle_selection(&mut self) {
        if let Some(branch) = self.selected_branch() {
            let name = branch.name.clone();
            if self.selected_branches.contains(&name) {
                self.selected_branches.remove(&name);
            } else {
                self.selected_branches.insert(name);
            }
        }
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if self.filtered_len() > 0 && self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        let filtered_len = self.filtered_len();
        if filtered_len > 0 && self.selected < filtered_len - 1 {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    /// Page up
    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.ensure_visible();
    }

    /// Page down
    pub fn page_down(&mut self, page_size: usize) {
        let filtered_len = self.filtered_len();
        if filtered_len > 0 {
            self.selected = (self.selected + page_size).min(filtered_len - 1);
            self.ensure_visible();
        }
    }

    /// Go to start
    pub fn go_home(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    /// Go to end
    pub fn go_end(&mut self) {
        let filtered_len = self.filtered_len();
        if filtered_len > 0 {
            self.selected = filtered_len - 1;
        }
        self.ensure_visible();
    }

    /// Ensure selected item is visible within the viewport
    fn ensure_visible(&mut self) {
        let visible_window = self.visible_height.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + visible_window {
            self.offset = self.selected.saturating_sub(visible_window - 1);
        }
    }

    /// Update visible height and re-adjust scroll position
    /// Should be called by renderer when viewport size is known
    pub fn update_visible_height(&mut self, height: usize) {
        if self.visible_height != height {
            self.visible_height = height;
            self.ensure_visible();
        }
    }

    /// Get currently selected branch
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        self.filtered_branch_at(self.selected)
    }

    /// Update running agents map from pane list
    pub fn update_running_agents(&mut self, panes: &[AgentPane]) {
        self.running_agents.clear();
        for pane in panes {
            self.running_agents
                .insert(pane.branch_name.clone(), pane.clone());
        }
    }

    /// Get running agent for a branch
    pub fn get_running_agent(&self, branch_name: &str) -> Option<&AgentPane> {
        self.running_agents.get(branch_name)
    }

    /// Update filter and reset selection
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
        self.offset = 0;
        self.rebuild_filtered_cache();
    }

    pub fn apply_pr_titles(&mut self, titles: &HashMap<String, String>) {
        if titles.is_empty() {
            return;
        }

        for item in &mut self.branches {
            if let Some(title) = titles.get(&item.name) {
                item.pr_title = Some(title.clone());
            }
        }

        self.rebuild_filtered_cache();
    }

    pub fn apply_safety_update(
        &mut self,
        branch_name: &str,
        has_unpushed: bool,
        is_unmerged: bool,
        safe_to_cleanup: bool,
    ) {
        if let Some(item) = self.branches.iter_mut().find(|b| b.name == branch_name) {
            if item.safe_to_cleanup.is_none() {
                item.has_unpushed = has_unpushed;
                item.is_unmerged = is_unmerged;
                item.safe_to_cleanup = Some(safe_to_cleanup);
                item.update_safety_status();
            }
        }
    }

    pub fn apply_worktree_update(
        &mut self,
        branch_name: &str,
        worktree_status: WorktreeStatus,
        has_changes: bool,
    ) {
        if let Some(item) = self.branches.iter_mut().find(|b| b.name == branch_name) {
            let prev_changes = item.has_changes;
            item.worktree_status = worktree_status;
            item.has_changes = has_changes;
            item.update_safety_status();

            if prev_changes != has_changes {
                if has_changes {
                    self.stats.changes_count = self.stats.changes_count.saturating_add(1);
                } else {
                    self.stats.changes_count = self.stats.changes_count.saturating_sub(1);
                }
            }
        }
    }

    pub fn reset_status_progress(&mut self, total: usize) {
        self.status_progress_total = total;
        self.status_progress_done = 0;
        self.status_progress_active = total > 0;
    }

    pub fn increment_status_progress(&mut self) {
        if !self.status_progress_active {
            return;
        }
        if self.status_progress_done < self.status_progress_total {
            self.status_progress_done += 1;
        }
        if self.status_progress_done >= self.status_progress_total {
            self.status_progress_active = false;
        }
    }

    pub fn status_progress_line(&self) -> Option<String> {
        if !self.status_progress_active {
            return None;
        }
        Some(format!(
            "Status: Updating branch status ({}/{})",
            self.status_progress_done, self.status_progress_total
        ))
    }

    /// Set loading state
    pub fn set_loading(&mut self, loading: bool) {
        self.is_loading = loading;
        if loading {
            self.loading_started = Some(Instant::now());
        } else {
            self.loading_started = None;
        }
    }

    /// Advance spinner frame (call on tick)
    pub fn tick_spinner(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// Get current spinner character
    pub fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    /// Check if loading indicator should be visible (after delay)
    pub fn should_show_spinner(&self, delay_ms: u64) -> bool {
        if !self.is_loading {
            return false;
        }
        if let Some(started) = self.loading_started {
            started.elapsed().as_millis() >= delay_ms as u128
        } else {
            false
        }
    }

    /// Update branch summary for the currently selected branch (SPEC-4b893dae T206)
    ///
    /// Fetches commit log and change stats from the repository.
    /// Should be called when selection changes.
    pub fn update_branch_summary(&mut self, repo_root: &Path) {
        let Some(branch) = self.selected_branch() else {
            self.branch_summary = None;
            return;
        };

        // Create base summary
        let mut summary = BranchSummary::new(&branch.name);

        // Set worktree path if available
        if let Some(wt_path) = &branch.worktree_path {
            summary = summary.with_worktree_path(Some(std::path::PathBuf::from(wt_path)));
        }

        // Try to fetch commit log
        // For branches with worktree, use the worktree path; otherwise use repo root
        let repo_path = branch
            .worktree_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| repo_root.to_path_buf());

        if let Ok(repo) = Repository::open(&repo_path) {
            // Fetch commit log (max 5 commits)
            match repo.get_commit_log(5) {
                Ok(commits) => {
                    summary = summary.with_commits(commits);
                }
                Err(e) => {
                    summary.errors.commits = Some(e.to_string());
                }
            }

            // Fetch change stats only if worktree exists
            if branch.worktree_path.is_some() {
                match repo.get_diff_stats() {
                    Ok(mut stats) => {
                        // Integrate existing safety check data
                        stats.has_uncommitted = branch.has_changes;
                        stats.has_unpushed = branch.has_unpushed;
                        summary = summary.with_stats(stats);
                    }
                    Err(e) => {
                        summary.errors.stats = Some(e.to_string());
                    }
                }
            }
        }

        // Set metadata from branch data
        // Extract ahead/behind from DivergenceStatus
        let (ahead, behind) = match &branch.divergence {
            DivergenceStatus::Ahead(a) => (*a, 0),
            DivergenceStatus::Behind(b) => (0, *b),
            DivergenceStatus::Diverged { ahead, behind } => (*ahead, *behind),
            DivergenceStatus::UpToDate | DivergenceStatus::NoRemote => (0, 0),
        };

        let meta = BranchMeta {
            upstream: branch.remote_name.clone(),
            ahead,
            behind,
            last_commit_timestamp: branch.last_commit_timestamp,
            base_branch: None,
        };
        summary = summary.with_meta(meta);

        self.branch_summary = Some(summary);
    }

    /// Clear branch summary (called when branches are reloaded)
    pub fn clear_branch_summary(&mut self) {
        self.branch_summary = None;
    }
}

/// Render branch list screen
/// Note: Header, Filter, Mode are rendered by app.rs view_boxed_header
/// This function only renders: BranchList + WorktreePath/Status
pub fn render_branch_list(
    state: &mut BranchListState,
    frame: &mut Frame,
    area: Rect,
    status_message: Option<&str>,
    has_focus: bool,
) {
    // SPEC-4b893dae: Summary panel height is 12 lines (FR-003)
    let panel_height = crate::tui::components::SummaryPanel::height();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),               // Branch list (FR-003)
            Constraint::Length(panel_height), // Summary panel (SPEC-4b893dae)
        ])
        .split(area);

    // Calculate visible height from branch list area (accounting for border)
    let branch_area_height = chunks[0].height.saturating_sub(2) as usize; // -2 for borders
    state.update_visible_height(branch_area_height);

    render_branches(state, frame, chunks[0], has_focus);
    render_summary_panel(state, frame, chunks[1], status_message);
}

/// Render header line (FR-001, FR-001a)
fn render_header(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let title = "GWT - Git Worktree Manager";
    let version = state.version.as_deref().unwrap_or("dev");
    let working_dir = state.working_directory.as_deref().unwrap_or(".");

    let mut spans = vec![
        Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{}", version),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(working_dir, Style::default().fg(Color::White)),
    ];

    // Add profile info if available (FR-001a)
    if let Some(profile) = &state.active_profile {
        spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            "Profile(p): ",
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(profile, Style::default().fg(Color::Yellow)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render filter line
fn render_filter_line(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let filtered_len = state.filtered_len();
    let total = state.branches.len();

    let mut spans = vec![Span::styled(
        "Filter(f): ",
        Style::default().fg(Color::DarkGray),
    )];

    if state.filter_mode {
        if state.filter.is_empty() {
            spans.push(Span::styled(
                "Type to search...",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(&state.filter));
        }
        spans.push(Span::styled("|", Style::default().fg(Color::White)));
    } else {
        spans.push(Span::styled(
            if state.filter.is_empty() {
                "(press f to filter)"
            } else {
                &state.filter
            },
            Style::default().fg(Color::DarkGray),
        ));
    }

    if !state.filter.is_empty() {
        spans.push(Span::styled(
            format!(" (Showing {} of {})", filtered_len, total),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render mode line
fn render_stats_line(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let spans = vec![
        Span::styled("Mode(m): ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.view_mode.label(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render branches list
fn render_branches(state: &BranchListState, frame: &mut Frame, area: Rect, has_focus: bool) {
    let border_style = if has_focus {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    frame.render_widget(block.clone(), area);
    let inner_area = block.inner(area);
    if inner_area.width == 0 || inner_area.height == 0 {
        return;
    }

    let filtered_len = state.filtered_len();

    // Show loading spinner when loading and branches are empty
    if filtered_len == 0 {
        if state.should_show_spinner(300) {
            // Show animated spinner after 300ms delay
            let spinner = state.spinner_char();
            let text = format!("{} Loading Git information...", spinner);
            let paragraph = Paragraph::new(text)
                .style(Style::default().fg(Color::Yellow))
                .alignment(Alignment::Center);
            frame.render_widget(paragraph, inner_area);
        } else if state.is_loading {
            // Before delay, show simple message
            let paragraph = Paragraph::new("Loading...")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            frame.render_widget(paragraph, inner_area);
        } else if state.filter.is_empty() {
            let paragraph = Paragraph::new("No branches found")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            frame.render_widget(paragraph, inner_area);
        } else {
            let paragraph = Paragraph::new("No branches match your filter")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            frame.render_widget(paragraph, inner_area);
        }
        return;
    }

    let visible_height = inner_area.height as usize;
    // FR-031b: Pass spinner_frame for safety check pending indicator
    let spinner_frame = state.spinner_frame;
    let mut items: Vec<ListItem> = state
        .visible_filtered_indices(visible_height)
        .iter()
        .enumerate()
        .map(|(i, index)| {
            let branch = &state.branches[*index];
            let running_agent = state.get_running_agent(&branch.name);
            render_branch_row(
                branch,
                state.offset + i == state.selected,
                &state.selected_branches,
                spinner_frame,
                running_agent,
                inner_area.width,
            )
        })
        .collect();

    if state.is_loading && items.len() < visible_height {
        let spinner = state.spinner_char();
        let text = format!("{} Loading more...", spinner);
        items.push(ListItem::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let list = List::new(items);
    frame.render_widget(list, inner_area);

    // Scrollbar
    if filtered_len > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("^"))
            .end_symbol(Some("v"));
        let mut scrollbar_state = ScrollbarState::new(filtered_len).position(state.selected);
        frame.render_stateful_widget(scrollbar, inner_area, &mut scrollbar_state);
    }
}

/// Render a single branch row
/// FR-070: Tool display format: ToolName@X.Y.Z
/// FR-031b: Show spinner for safety check pending branches
/// FR-020~024: Running agent info displayed on right side (right-aligned)
fn render_branch_row(
    branch: &BranchItem,
    is_selected: bool,
    selected_set: &HashSet<String>,
    spinner_frame: usize,
    running_agent: Option<&AgentPane>,
    width: u16,
) -> ListItem<'static> {
    // Only show selection icons when at least one branch is selected
    let show_selection = !selected_set.is_empty();
    let is_checked = selected_set.contains(&branch.name);
    let (worktree_icon, worktree_color) = branch.worktree_icon();
    // FR-031b: Pass spinner_frame for pending safety check
    let (safety_icon, safety_color) = branch.safety_icon(Some(spinner_frame));

    // Branch name
    let display_name = if branch.branch_type == BranchType::Remote {
        branch.remote_name.as_deref().unwrap_or(&branch.name)
    } else {
        &branch.name
    };

    // Calculate left side width: optionally "[*] " + worktree + " " + safety + " " + branch_name
    // selection_icon(3) + space(1) if showing selection, plus worktree_icon(1) + space(1) + safety_icon + space(1) + name
    let selection_width = if show_selection { 3 } else { 0 }; // "◉ " or "◎ " (2 + 1)
    let left_width = selection_width + 1 + 1 + safety_icon.len() + 1 + display_name.width();

    // Build right side (agent info) and calculate its width
    // SPEC-861d8cdf T-103: Status-based display
    let (right_spans, right_width): (Vec<Span>, usize) = if let Some(agent) = running_agent {
        let agent_display = get_agent_display_name(&agent.agent_name);
        let uptime = agent.uptime_string();

        // Determine icon and color based on status (SPEC-861d8cdf T-103)
        let (status_icon, status_color) = match agent.status {
            AgentStatus::Running => {
                if agent.is_background {
                    let icon = BG_SPINNER_FRAMES[spinner_frame % BG_SPINNER_FRAMES.len()];
                    (icon, Color::DarkGray)
                } else {
                    let icon = ACTIVE_SPINNER_FRAMES[spinner_frame % ACTIVE_SPINNER_FRAMES.len()];
                    (icon, Color::Green)
                }
            }
            AgentStatus::WaitingInput => {
                // Blink effect: 500ms on/off (2 spinner frames = ~500ms with 250ms tick)
                let should_show = (spinner_frame / 2) % 2 == 0;
                if should_show {
                    ('?', Color::Yellow)
                } else {
                    (' ', Color::Yellow)
                }
            }
            AgentStatus::Stopped => ('#', Color::Red),
            AgentStatus::Unknown => {
                // Fallback to original behavior based on is_background
                if agent.is_background {
                    let icon = BG_SPINNER_FRAMES[spinner_frame % BG_SPINNER_FRAMES.len()];
                    (icon, Color::DarkGray)
                } else {
                    let icon = ACTIVE_SPINNER_FRAMES[spinner_frame % ACTIVE_SPINNER_FRAMES.len()];
                    (icon, Color::Green)
                }
            }
        };

        // Determine text color based on status
        let text_color = match agent.status {
            AgentStatus::Stopped => Color::Red,
            AgentStatus::WaitingInput => Color::Yellow,
            AgentStatus::Running if agent.is_background => Color::DarkGray,
            _ => get_agent_color(Some(&agent.agent_name)),
        };

        let uptime_color = match agent.status {
            AgentStatus::Stopped => Color::DarkGray,
            AgentStatus::WaitingInput => Color::Yellow,
            AgentStatus::Running if agent.is_background => Color::DarkGray,
            _ => Color::Yellow,
        };

        let width = 2 + agent_display.width() + 1 + uptime.width();
        let spans = vec![
            Span::styled(format!("{} ", status_icon), Style::default().fg(status_color)),
            Span::styled(agent_display, Style::default().fg(text_color)),
            Span::raw(" "),
            Span::styled(uptime, Style::default().fg(uptime_color)),
        ];
        (spans, width)
    } else if let Some(tool) = &branch.last_tool_usage {
        // No running agent, but show last tool usage (FR-070)
        let agent_id = tool.split('@').next();
        let agent_color = get_agent_color(agent_id);
        let spans = vec![Span::styled(
            tool.to_string(),
            Style::default().fg(agent_color),
        )];
        (spans, tool.width())
    } else {
        (vec![], 0)
    };

    // Calculate padding between left and right sides
    let total_content = left_width + right_width;
    let available = width as usize;
    let padding = if total_content < available && right_width > 0 {
        available.saturating_sub(total_content).saturating_sub(1) // -1 for space before right side
    } else {
        1 // minimum single space
    };

    // Build the complete spans
    let mut spans = Vec::new();

    // Only add selection icon when in selection mode
    if show_selection {
        let selection_icon = if is_checked { "◉" } else { "◎" };
        spans.push(Span::styled(
            selection_icon,
            if is_checked && (branch.has_changes || branch.has_unpushed) {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            },
        ));
        spans.push(Span::raw(" "));
    }

    spans.extend([
        Span::styled(worktree_icon, Style::default().fg(worktree_color)),
        Span::raw(" "),
        Span::styled(safety_icon, Style::default().fg(safety_color)),
        Span::raw(" "),
        Span::raw(display_name.to_string()),
    ]);

    // Add padding and right side if there's agent info
    if !right_spans.is_empty() {
        spans.push(Span::raw(" ".repeat(padding)));
        spans.extend(right_spans);
    }

    // FR-018: Selected branch shown with cyan background
    let style = if is_selected {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

/// Render summary panel (SPEC-4b893dae FR-001~FR-006)
fn render_summary_panel(
    state: &BranchListState,
    frame: &mut Frame,
    area: Rect,
    status_message: Option<&str>,
) {
    use crate::tui::components::SummaryPanel;
    use std::path::PathBuf;

    // Create or get branch summary
    let summary = if let Some(ref summary) = state.branch_summary {
        summary.clone()
    } else if let Some(branch) = state.selected_branch() {
        // Create a basic summary from available branch data
        let mut summary = BranchSummary::new(&branch.name);

        // Set worktree path if available
        if let Some(wt_path) = &branch.worktree_path {
            summary = summary.with_worktree_path(Some(PathBuf::from(wt_path)));
        }

        // Set loading state based on global loading state
        if state.is_loading {
            summary.loading.commits = true;
            summary.loading.stats = true;
            summary.loading.meta = true;
        }

        summary
    } else {
        // No branch selected - show empty panel
        BranchSummary::new("(no branch selected)")
    };

    // Handle status messages - show them in the panel area if present
    if let Some(status) = status_message {
        // Draw the panel frame first
        let title = format!(" [{}] Details ", summary.branch_name);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Show status message inside
        let line = Line::from(vec![Span::styled(
            status,
            Style::default().fg(Color::Yellow),
        )]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    }

    // Handle loading state - show spinner in panel
    if state.is_loading {
        let title = format!(" [{}] Details ", summary.branch_name);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let line = Line::from(vec![
            Span::styled(
                format!("{} ", state.spinner_char()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                "Loading branch information...",
                Style::default().fg(Color::Yellow),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    }

    // Handle progress state
    if let Some(progress) = state.status_progress_line() {
        let title = format!(" [{}] Details ", summary.branch_name);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let line = Line::from(vec![
            Span::styled(
                format!("{} ", state.spinner_char()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(progress, Style::default().fg(Color::Yellow)),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    }

    // Render the full summary panel
    let panel = SummaryPanel::new(&summary)
        .with_tick(state.spinner_frame)
        .with_ai_enabled(false); // AI will be enabled in Phase 7

    panel.render(frame, area);
}

/// Render footer line with keybindings (FR-004)
fn render_footer(frame: &mut Frame, area: Rect) {
    let keybinds = vec![
        ("r", "Refresh"),
        ("c", "Cleanup"),
        ("x", "Repair"),
        ("l", "Logs"),
        ("p", "Profile"),
        ("f", "Filter"),
        ("m", "Mode"),
        ("?", "Help"),
        ("q", "Quit"),
    ];

    let mut spans = Vec::new();
    for (i, (key, action)) in keybinds.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", Style::default()));
        }
        spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(*key, Style::default().fg(Color::Cyan)));
        spans.push(Span::styled(":", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(*action, Style::default().fg(Color::White)));
        spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_view_mode_cycle() {
        assert_eq!(ViewMode::All.cycle(), ViewMode::Local);
        assert_eq!(ViewMode::Local.cycle(), ViewMode::Remote);
        assert_eq!(ViewMode::Remote.cycle(), ViewMode::All);
    }

    #[test]
    fn test_spinner_char_wraps_large_frame() {
        let mut state = BranchListState::new();
        state.spinner_frame = SPINNER_FRAMES.len() * 3 + 1;
        assert_eq!(state.spinner_char(), '/');
        state.spinner_frame = SPINNER_FRAMES.len() * 5;
        assert_eq!(state.spinner_char(), '|');
    }

    #[test]
    fn test_branch_list_navigation() {
        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                branch_type: BranchType::Local,
                is_current: true,
                has_worktree: true,
                worktree_path: Some("/path".to_string()),
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "develop".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: None,
                safety_status: SafetyStatus::Pending,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_render_branches_draws_border() {
        let branches = vec![BranchItem {
            name: "main".to_string(),
            branch_type: BranchType::Local,
            is_current: true,
            has_worktree: true,
            worktree_path: Some("/path".to_string()),
            worktree_status: WorktreeStatus::Active,
            has_changes: false,
            has_unpushed: false,
            divergence: DivergenceStatus::UpToDate,
            has_remote_counterpart: true,
            remote_name: None,
            safe_to_cleanup: Some(true),
            safety_status: SafetyStatus::Safe,
            is_unmerged: false,
            last_commit_timestamp: None,
            last_tool_usage: None,
            is_selected: false,
            pr_title: None,
        }];

        let mut state = BranchListState::new().with_branches(branches);
        state.update_visible_height(3); // 5 - 2 for borders
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branches(&state, f, area, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(19, 0)].symbol(), "┐");
        assert_eq!(buffer[(0, 4)].symbol(), "└");
        assert_eq!(buffer[(19, 4)].symbol(), "┘");
    }

    #[test]
    fn test_status_progress_line_renders() {
        let branches = vec![BranchItem {
            name: "main".to_string(),
            branch_type: BranchType::Local,
            is_current: true,
            has_worktree: true,
            worktree_path: Some("/path".to_string()),
            worktree_status: WorktreeStatus::Active,
            has_changes: false,
            has_unpushed: false,
            divergence: DivergenceStatus::UpToDate,
            has_remote_counterpart: true,
            remote_name: None,
            safe_to_cleanup: Some(true),
            safety_status: SafetyStatus::Safe,
            is_unmerged: false,
            last_commit_timestamp: None,
            last_tool_usage: None,
            is_selected: false,
            pr_title: None,
        }];

        let mut state = BranchListState::new().with_branches(branches);
        state.reset_status_progress(5);
        state.increment_status_progress();

        // SPEC-4b893dae: Panel is 12 lines, so need taller terminal (3 min + 12 panel = 15)
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branch_list(&mut state, f, area, None, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        // Search the entire buffer for the progress text (now inside panel)
        let mut found = false;
        for y in 0..20 {
            let line: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if line.contains("Updating branch status (1/5)") {
                found = true;
                break;
            }
        }
        assert!(found, "Progress line should appear somewhere in the panel");
    }

    #[test]
    fn test_loading_status_line_renders() {
        let mut state = BranchListState::new();
        state.set_loading(true);

        // SPEC-4b893dae: Panel is 12 lines, so need taller terminal (3 min + 12 panel = 15)
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branch_list(&mut state, f, area, None, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        // Search the entire buffer for the loading text (now inside panel)
        let mut found = false;
        for y in 0..20 {
            let line: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if line.contains("Loading branch information") {
                found = true;
                break;
            }
        }
        assert!(found, "Loading text should appear somewhere in the panel");
    }

    #[test]
    fn test_view_mode_filter() {
        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                branch_type: BranchType::Local,
                is_current: true,
                has_worktree: true,
                worktree_path: None,
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "remotes/origin/main".to_string(),
                branch_type: BranchType::Remote,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: Some("remotes/origin/main".to_string()),
                safe_to_cleanup: None,
                safety_status: SafetyStatus::Unknown,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);

        assert_eq!(state.filtered_branches().len(), 2);

        state.set_view_mode(ViewMode::Local);
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "main");

        state.set_view_mode(ViewMode::Remote);
        assert_eq!(state.filtered_branches().len(), 2); // main has remote counterpart
    }

    #[test]
    fn test_filter_cache_rebuilds_only_on_filter_or_mode_change() {
        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                branch_type: BranchType::Local,
                is_current: true,
                has_worktree: true,
                worktree_path: None,
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "feature/one".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        let initial_version = state.filter_cache_version;

        state.select_next();
        assert_eq!(state.filter_cache_version, initial_version);

        state.set_filter("main".to_string());
        assert!(state.filter_cache_version > initial_version);

        let after_filter = state.filter_cache_version;
        state.cycle_view_mode();
        assert!(state.filter_cache_version > after_filter);
    }

    #[test]
    fn test_apply_pr_titles_updates_filter_results() {
        let branches = vec![
            BranchItem {
                name: "feature/one".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "feature/two".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        state.set_filter("cool".to_string());
        assert_eq!(state.filtered_branches().len(), 0);

        let mut titles = HashMap::new();
        titles.insert("feature/one".to_string(), "Cool PR".to_string());
        state.apply_pr_titles(&titles);

        let filtered = state.filtered_branches();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "feature/one");
    }

    #[test]
    fn test_update_safety_status_prioritizes_flags() {
        let mut item = BranchItem {
            name: "feature/one".to_string(),
            branch_type: BranchType::Local,
            is_current: false,
            has_worktree: false,
            worktree_path: None,
            worktree_status: WorktreeStatus::None,
            has_changes: true,
            has_unpushed: true,
            divergence: DivergenceStatus::UpToDate,
            has_remote_counterpart: false,
            remote_name: None,
            safe_to_cleanup: Some(true),
            safety_status: SafetyStatus::Unknown,
            is_unmerged: true,
            last_commit_timestamp: None,
            last_tool_usage: None,
            is_selected: false,
            pr_title: None,
        };

        item.update_safety_status();
        assert_eq!(item.safety_status, SafetyStatus::Uncommitted);

        item.has_changes = false;
        item.update_safety_status();
        assert_eq!(item.safety_status, SafetyStatus::Unpushed);

        item.has_unpushed = false;
        item.update_safety_status();
        assert_eq!(item.safety_status, SafetyStatus::Unmerged);

        item.is_unmerged = false;
        item.safe_to_cleanup = Some(true);
        item.update_safety_status();
        assert_eq!(item.safety_status, SafetyStatus::Safe);

        item.safe_to_cleanup = None;
        item.update_safety_status();
        assert_eq!(item.safety_status, SafetyStatus::Pending);

        item.safe_to_cleanup = Some(false);
        item.update_safety_status();
        assert_eq!(item.safety_status, SafetyStatus::Unsafe);
    }

    #[test]
    fn test_apply_safety_update_updates_pending_branch() {
        let branches = vec![
            BranchItem {
                name: "feature/one".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: None,
                safety_status: SafetyStatus::Pending,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "feature/two".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: Some(false),
                safety_status: SafetyStatus::Unsafe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        state.apply_safety_update("feature/one", true, false, false);

        let item = state
            .branches
            .iter()
            .find(|b| b.name == "feature/one")
            .unwrap();
        assert!(item.has_unpushed);
        assert_eq!(item.safe_to_cleanup, Some(false));
        assert!(!item.is_unmerged);
        assert_eq!(item.safety_status, SafetyStatus::Unpushed);

        state.apply_safety_update("feature/two", false, false, true);
        let item = state
            .branches
            .iter()
            .find(|b| b.name == "feature/two")
            .unwrap();
        assert_eq!(item.safe_to_cleanup, Some(false));
        assert_eq!(item.safety_status, SafetyStatus::Unsafe);
    }

    #[test]
    fn test_visible_filtered_indices_respects_offset_and_height() {
        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                branch_type: BranchType::Local,
                is_current: true,
                has_worktree: true,
                worktree_path: None,
                worktree_status: WorktreeStatus::Active,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: true,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "feature/one".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
            BranchItem {
                name: "feature/two".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
                has_remote_counterpart: false,
                remote_name: None,
                safe_to_cleanup: Some(true),
                safety_status: SafetyStatus::Safe,
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
                pr_title: None,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        state.offset = 1;
        let visible = state.visible_filtered_indices(2);
        assert_eq!(visible.len(), 2);
        assert_eq!(state.branches[visible[0]].name, "feature/one");
        assert_eq!(state.branches[visible[1]].name, "feature/two");
    }
}
