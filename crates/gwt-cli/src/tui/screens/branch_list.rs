//! Branch List Screen - TypeScript版完全互換

#![allow(dead_code)]

use crate::tui::components::LinkRegion;
use gwt_core::ai::SessionSummaryCache;
use gwt_core::config::AgentStatus;
use gwt_core::git::{Branch, BranchMeta, BranchSummary, DivergenceStatus, Repository};
use gwt_core::tmux::{AgentPane, StatusBarSummary};
use gwt_core::worktree::Worktree;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::{prelude::*, widgets::*};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

const BRANCH_LIST_PADDING_X: u16 = 1;
const PANEL_PADDING_X: u16 = 1;

/// Get branch name type for sorting
fn get_branch_name_type(name: &str) -> BranchNameType {
    let lower = name.to_lowercase();
    // Strip only the "remotes/<remote>/" prefix for comparison
    let short_name = if let Some(stripped) = lower.strip_prefix("remotes/") {
        stripped
            .split_once('/')
            .map(|(_, rest)| rest)
            .unwrap_or(stripped)
    } else {
        lower.as_str()
    };

    if short_name == "main" || short_name == "master" {
        BranchNameType::Main
    } else if short_name == "develop" || short_name == "dev" {
        BranchNameType::Develop
    } else if short_name.starts_with("feature/") {
        BranchNameType::Feature
    } else if short_name.starts_with("bugfix/") || short_name.starts_with("bug/") {
        BranchNameType::Bugfix
    } else if short_name.starts_with("hotfix/") {
        BranchNameType::Hotfix
    } else if short_name.starts_with("release/") {
        BranchNameType::Release
    } else {
        BranchNameType::Other
    }
}

/// View mode for branch list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    All,
    #[default]
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

/// Sort mode for branch list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BranchSortMode {
    #[default]
    Default,
    Name,
    Updated,
}

impl BranchSortMode {
    pub fn label(&self) -> &'static str {
        match self {
            BranchSortMode::Default => "Default",
            BranchSortMode::Name => "Name",
            BranchSortMode::Updated => "Updated",
        }
    }

    pub fn cycle(&self) -> Self {
        match self {
            BranchSortMode::Default => BranchSortMode::Name,
            BranchSortMode::Name => BranchSortMode::Updated,
            BranchSortMode::Updated => BranchSortMode::Default,
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
    pub last_tool_id: Option<String>,
    pub last_session_id: Option<String>,
    pub is_selected: bool,
    /// PR title for search (FR-016)
    pub pr_title: Option<String>,
    /// PR number for latest PR (if any)
    pub pr_number: Option<u64>,
    /// PR URL for latest PR (if any)
    pub pr_url: Option<String>,
    /// PR state for latest PR (if any)
    pub pr_state: Option<String>,
    /// FR-085: Whether the upstream branch has been deleted (gone)
    pub is_gone: bool,
}

/// PR info mapped to branch name
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub title: String,
    pub number: u64,
    pub url: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct BranchSummaryRequest {
    pub branch: String,
    pub repo_root: PathBuf,
    pub branch_item: BranchItem,
}

#[derive(Debug, Clone)]
pub struct BranchSummaryUpdate {
    pub branch: String,
    pub summary: BranchSummary,
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
            has_remote_counterpart: if branch_type == BranchType::Local {
                branch.has_remote
            } else {
                false
            },
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None, // FR-016: Will be populated from PrCache
            pr_state: None,
            is_gone: branch.is_gone, // FR-085: Populate gone status from Branch
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
            has_remote_counterpart: if branch_type == BranchType::Local {
                branch.has_remote
            } else {
                false
            },
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: branch.is_gone, // FR-085: Populate gone status from Branch
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

    /// FR-082/FR-083/FR-084/FR-085: Get branch name color based on worktree status and gone status
    /// - White: Worktree exists and active
    /// - DarkGray: No Worktree
    /// - Red: Worktree path inaccessible OR branch is gone (upstream deleted)
    pub fn branch_name_color(&self) -> Color {
        if self.is_gone {
            // FR-085: Gone branch (upstream deleted) shown in red
            Color::Red
        } else {
            match self.worktree_status {
                WorktreeStatus::Active => Color::White, // FR-082: Active worktree
                WorktreeStatus::None => Color::DarkGray, // FR-083: No worktree
                WorktreeStatus::Inaccessible => Color::Red, // FR-084: Inaccessible path
            }
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
    pub sort_mode: BranchSortMode,
    pub version: Option<String>,
    pub working_directory: Option<String>,
    pub active_profile: Option<String>,
    pub spinner_frame: usize,
    pub filter_cache_version: u64,
    pub status_progress_total: usize,
    pub status_progress_done: usize,
    pub status_progress_active: bool,
    pub cleanup_in_progress: bool,
    pub cleanup_progress_total: usize,
    pub cleanup_progress_done: usize,
    pub cleanup_active_branch: Option<String>,
    cleanup_target_branches: HashSet<String>,
    /// Viewport height for scroll calculations (updated by renderer)
    pub visible_height: usize,
    /// Cached branch list area (outer, with border)
    pub list_area: Option<Rect>,
    /// Cached branch list inner area (content rows)
    pub list_inner_area: Option<Rect>,
    /// Running agents mapped by branch name (for agent info display)
    pub running_agents: HashMap<String, AgentPane>,
    /// Branch summary data for the selected branch (SPEC-4b893dae)
    pub branch_summary: Option<BranchSummary>,
    /// AI settings enabled for active profile
    pub ai_enabled: bool,
    /// Session summary cache (session)
    session_summary_cache: SessionSummaryCache,
    /// Session summary requests in-flight
    session_summary_inflight: HashSet<String>,
    /// Session missing for branch
    session_missing: HashSet<String>,
    /// Session summary warnings per branch
    session_summary_warnings: HashMap<String, String>,
    /// Session summary scroll offset
    session_scroll_offset: usize,
    /// Session summary scroll max
    session_scroll_max: usize,
    /// Session summary scroll page size
    session_scroll_page: usize,
    /// Cached session panel area (outer, with border)
    session_panel_area: Option<Rect>,
    /// Cached session panel inner area (content rows)
    session_panel_inner_area: Option<Rect>,
    /// Cached repo web URL for GitHub links
    repo_web_url: Option<String>,
    /// Clickable link regions in details panel
    detail_links: Vec<LinkRegion>,
}

#[derive(Debug, Clone)]
pub struct CleanupStateSnapshot {
    pub in_progress: bool,
    pub progress_total: usize,
    pub progress_done: usize,
    pub active_branch: Option<String>,
    pub target_branches: Vec<String>,
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
            sort_mode: BranchSortMode::default(),
            version: None,
            working_directory: None,
            active_profile: None,
            spinner_frame: 0,
            filter_cache_version: 0,
            status_progress_total: 0,
            status_progress_done: 0,
            status_progress_active: false,
            cleanup_in_progress: false,
            cleanup_progress_total: 0,
            cleanup_progress_done: 0,
            cleanup_active_branch: None,
            cleanup_target_branches: HashSet::new(),
            visible_height: 15, // Default fallback (previously hardcoded)
            list_area: None,
            list_inner_area: None,
            running_agents: HashMap::new(),
            branch_summary: None,
            ai_enabled: false,
            session_summary_cache: SessionSummaryCache::default(),
            session_summary_inflight: HashSet::new(),
            session_missing: HashSet::new(),
            session_summary_warnings: HashMap::new(),
            session_scroll_offset: 0,
            session_scroll_max: 0,
            session_scroll_page: 0,
            session_panel_area: None,
            session_panel_inner_area: None,
            repo_web_url: None,
            detail_links: Vec::new(),
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
                .filter(|b| b.branch_type == BranchType::Remote)
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
                ViewMode::Remote => branch.branch_type == BranchType::Remote,
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

        result.sort_by(|&a_index, &b_index| {
            let a = &self.branches[a_index];
            let b = &self.branches[b_index];

            let compare_current = |a: &BranchItem, b: &BranchItem| -> Option<Ordering> {
                if a.is_current && !b.is_current {
                    Some(Ordering::Less)
                } else if !a.is_current && b.is_current {
                    Some(Ordering::Greater)
                } else {
                    None
                }
            };

            let compare_timestamp = |a: &BranchItem, b: &BranchItem| -> Option<Ordering> {
                match (a.last_commit_timestamp, b.last_commit_timestamp) {
                    (Some(a_time), Some(b_time)) => {
                        if a_time == b_time {
                            None
                        } else {
                            Some(b_time.cmp(&a_time))
                        }
                    }
                    (Some(_), None) => Some(Ordering::Less),
                    (None, Some(_)) => Some(Ordering::Greater),
                    (None, None) => None,
                }
            };

            let compare_local_remote = |a: &BranchItem, b: &BranchItem| -> Option<Ordering> {
                match (a.branch_type, b.branch_type) {
                    (BranchType::Local, BranchType::Remote) => Some(Ordering::Less),
                    (BranchType::Remote, BranchType::Local) => Some(Ordering::Greater),
                    _ => None,
                }
            };

            match self.sort_mode {
                BranchSortMode::Default => {
                    if let Some(ordering) = compare_current(a, b) {
                        return ordering;
                    }

                    let a_type = get_branch_name_type(&a.name);
                    let b_type = get_branch_name_type(&b.name);
                    if a_type != b_type {
                        return a_type.cmp(&b_type);
                    }

                    if a.has_worktree && !b.has_worktree {
                        return Ordering::Less;
                    }
                    if !a.has_worktree && b.has_worktree {
                        return Ordering::Greater;
                    }

                    if let Some(ordering) = compare_timestamp(a, b) {
                        return ordering;
                    }

                    if let Some(ordering) = compare_local_remote(a, b) {
                        return ordering;
                    }

                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                }
                BranchSortMode::Name => {
                    if let Some(ordering) = compare_current(a, b) {
                        return ordering;
                    }

                    let name_order = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                    if name_order != Ordering::Equal {
                        return name_order;
                    }

                    if let Some(ordering) = compare_local_remote(a, b) {
                        return ordering;
                    }

                    Ordering::Equal
                }
                BranchSortMode::Updated => {
                    if let Some(ordering) = compare_current(a, b) {
                        return ordering;
                    }

                    if let Some(ordering) = compare_timestamp(a, b) {
                        return ordering;
                    }

                    let name_order = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                    if name_order != Ordering::Equal {
                        return name_order;
                    }

                    if let Some(ordering) = compare_local_remote(a, b) {
                        return ordering;
                    }

                    Ordering::Equal
                }
            }
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

    fn rebuild_filtered_cache_preserve_selection(&mut self) {
        let selected_name = self.selected_branch().map(|branch| branch.name.clone());
        self.rebuild_filtered_cache();
        if let Some(name) = selected_name {
            if let Some(index) = self
                .filtered_indices
                .iter()
                .position(|&idx| self.branches[idx].name == name)
            {
                self.selected = index;
                self.ensure_visible();
            }
        }
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
    /// Sorted according to current sort mode (Default/Name/Updated).
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

    /// Cycle sort mode
    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.cycle();
        self.rebuild_filtered_cache_preserve_selection();
    }

    pub fn set_sort_mode(&mut self, sort_mode: BranchSortMode) {
        self.sort_mode = sort_mode;
        self.rebuild_filtered_cache_preserve_selection();
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
        if self.filtered_len() == 0 || self.selected == 0 {
            return;
        }
        let mut index = self.selected;
        while index > 0 {
            index -= 1;
            if !self.is_cleanup_target_index(index) {
                self.selected = index;
                self.ensure_visible();
                break;
            }
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        let filtered_len = self.filtered_len();
        if filtered_len == 0 || self.selected >= filtered_len - 1 {
            return;
        }
        let mut index = self.selected;
        while index + 1 < filtered_len {
            index += 1;
            if !self.is_cleanup_target_index(index) {
                self.selected = index;
                self.ensure_visible();
                break;
            }
        }
    }

    /// Page up
    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.move_selection_off_cleanup_target();
        self.ensure_visible();
    }

    /// Page down
    pub fn page_down(&mut self, page_size: usize) {
        let filtered_len = self.filtered_len();
        if filtered_len > 0 {
            self.selected = (self.selected + page_size).min(filtered_len - 1);
            self.move_selection_off_cleanup_target();
            self.ensure_visible();
        }
    }

    /// Go to start
    pub fn go_home(&mut self) {
        self.selected = 0;
        self.offset = 0;
        self.move_selection_off_cleanup_target();
    }

    /// Go to end
    pub fn go_end(&mut self) {
        let filtered_len = self.filtered_len();
        if filtered_len > 0 {
            self.selected = filtered_len - 1;
        }
        self.move_selection_off_cleanup_target();
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

    /// Update cached list areas based on rendered branch list block
    pub fn update_list_area(&mut self, area: Rect) {
        self.list_area = Some(area);
        let min_width = 2 + (BRANCH_LIST_PADDING_X * 2);
        let inner = if area.width <= min_width || area.height <= 2 {
            Rect {
                x: area.x,
                y: area.y,
                width: 0,
                height: 0,
            }
        } else {
            Rect {
                x: area.x.saturating_add(1 + BRANCH_LIST_PADDING_X),
                y: area.y.saturating_add(1),
                width: area.width.saturating_sub(2 + (BRANCH_LIST_PADDING_X * 2)),
                height: area.height.saturating_sub(2),
            }
        };
        self.list_inner_area = Some(inner);
        self.update_visible_height(inner.height as usize);
    }

    /// Update cached session panel areas based on rendered panel
    pub fn update_session_panel_area(&mut self, area: Rect, inner: Rect) {
        self.session_panel_area = Some(area);
        self.session_panel_inner_area = Some(inner);
    }

    /// Check if a point is inside the session panel content area
    pub fn session_panel_contains(&self, x: u16, y: u16) -> bool {
        let Some(inner) = self.session_panel_inner_area else {
            return false;
        };
        if inner.width == 0 || inner.height == 0 {
            return false;
        }
        let right = inner.x.saturating_add(inner.width);
        let bottom = inner.y.saturating_add(inner.height);
        x >= inner.x && x < right && y >= inner.y && y < bottom
    }

    /// Resolve selection index from a mouse position within the list area
    pub fn selection_index_from_point(&self, x: u16, y: u16) -> Option<usize> {
        let inner = self.list_inner_area?;
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        let right = inner.x.saturating_add(inner.width);
        let bottom = inner.y.saturating_add(inner.height);
        if x < inner.x || x >= right || y < inner.y || y >= bottom {
            return None;
        }
        let row = (y - inner.y) as usize;
        let index = self.offset.saturating_add(row);
        if index >= self.filtered_indices.len() {
            return None;
        }
        Some(index)
    }

    /// Set selected index directly (returns true if selection changed)
    pub fn select_index(&mut self, index: usize) -> bool {
        if index >= self.filtered_indices.len() {
            return false;
        }
        if self.is_cleanup_target_index(index) {
            return false;
        }
        if self.selected != index {
            self.selected = index;
            self.ensure_visible();
            return true;
        }
        false
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

    pub fn apply_pr_info(&mut self, info: &HashMap<String, PrInfo>) {
        if info.is_empty() {
            return;
        }

        for item in &mut self.branches {
            if let Some(pr) = info.get(&item.name) {
                item.pr_title = Some(pr.title.clone());
                item.pr_number = Some(pr.number);
                item.pr_url = pr.url.clone();
                item.pr_state = Some(pr.state.clone());
            }
        }

        self.rebuild_filtered_cache();
    }

    pub fn set_repo_web_url(&mut self, url: Option<String>) {
        self.repo_web_url = url;
    }

    pub fn repo_web_url(&self) -> Option<&String> {
        self.repo_web_url.as_ref()
    }

    pub fn set_detail_links(&mut self, links: Vec<LinkRegion>) {
        self.detail_links = links;
    }

    pub fn clear_detail_links(&mut self) {
        self.detail_links.clear();
    }

    pub fn link_at_point(&self, column: u16, row: u16) -> Option<String> {
        for link in &self.detail_links {
            if column >= link.area.x
                && column < link.area.x.saturating_add(link.area.width)
                && row >= link.area.y
                && row < link.area.y.saturating_add(link.area.height)
            {
                return Some(link.url.clone());
            }
        }
        None
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

    pub fn apply_worktree_created(&mut self, branch_name: &str, worktree_path: &Path) -> bool {
        let Some(item) = self.branches.iter_mut().find(|b| b.name == branch_name) else {
            return false;
        };

        item.has_worktree = true;
        item.worktree_path = Some(worktree_path.display().to_string());
        item.worktree_status = if worktree_path.exists() {
            WorktreeStatus::Active
        } else {
            WorktreeStatus::Inaccessible
        };
        // SPEC-a70a1ece FR-170: For bare repos, branch with worktree becomes Local
        if item.branch_type == BranchType::Remote {
            item.branch_type = BranchType::Local;
        }
        item.update_safety_status();

        self.stats.worktree_count = self.branches.iter().filter(|b| b.has_worktree).count();
        self.rebuild_filtered_cache_preserve_selection();
        true
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

    pub fn start_cleanup_progress(&mut self, total: usize) {
        self.cleanup_in_progress = total > 0;
        self.cleanup_progress_total = total;
        self.cleanup_progress_done = 0;
        self.cleanup_active_branch = None;
        self.cleanup_target_branches.clear();
    }

    pub fn cleanup_snapshot(&self) -> CleanupStateSnapshot {
        let mut target_branches: Vec<String> =
            self.cleanup_target_branches.iter().cloned().collect();
        target_branches.sort();
        CleanupStateSnapshot {
            in_progress: self.cleanup_in_progress,
            progress_total: self.cleanup_progress_total,
            progress_done: self.cleanup_progress_done,
            active_branch: self.cleanup_active_branch.clone(),
            target_branches,
        }
    }

    pub fn restore_cleanup_snapshot(&mut self, snapshot: &CleanupStateSnapshot) {
        if !snapshot.in_progress {
            self.finish_cleanup_progress();
            return;
        }
        self.cleanup_in_progress = true;
        self.cleanup_progress_total = snapshot.progress_total;
        self.cleanup_progress_done = snapshot.progress_done;
        self.cleanup_active_branch = snapshot.active_branch.clone();
        self.cleanup_target_branches.clear();
        self.cleanup_target_branches
            .extend(snapshot.target_branches.iter().cloned());
        self.move_selection_off_cleanup_target();
    }

    pub fn increment_cleanup_progress(&mut self) {
        if !self.cleanup_in_progress {
            return;
        }
        if self.cleanup_progress_done < self.cleanup_progress_total {
            self.cleanup_progress_done += 1;
        }
    }

    pub fn cleanup_progress_line(&self) -> Option<String> {
        if !self.cleanup_in_progress {
            return None;
        }
        Some(format!(
            "Cleanup: Running {} ({}/{})",
            self.spinner_char(),
            self.cleanup_progress_done,
            self.cleanup_progress_total
        ))
    }

    pub fn active_status_line(&self) -> Option<String> {
        self.cleanup_progress_line()
            .or_else(|| self.status_progress_line())
    }

    pub fn cleanup_in_progress(&self) -> bool {
        self.cleanup_in_progress
    }

    pub fn set_cleanup_target_branches(&mut self, branches: &[String]) {
        self.cleanup_target_branches.clear();
        self.cleanup_target_branches
            .extend(branches.iter().cloned());
        self.move_selection_off_cleanup_target();
    }

    pub fn set_cleanup_active_branch(&mut self, branch: Option<String>) {
        self.cleanup_active_branch = branch;
        self.move_selection_off_cleanup_target();
    }

    pub fn cleanup_active_branch(&self) -> Option<&str> {
        self.cleanup_active_branch.as_deref()
    }

    pub fn is_cleanup_target_index(&self, index: usize) -> bool {
        if !self.cleanup_in_progress || self.cleanup_target_branches.is_empty() {
            return false;
        }
        self.filtered_branch_at(index)
            .is_some_and(|branch| self.cleanup_target_branches.contains(&branch.name))
    }

    fn move_selection_off_cleanup_target(&mut self) {
        if !self.is_cleanup_target_index(self.selected) {
            return;
        }
        let filtered_len = self.filtered_len();
        if filtered_len == 0 {
            return;
        }
        for index in (self.selected + 1)..filtered_len {
            if !self.is_cleanup_target_index(index) {
                self.selected = index;
                self.ensure_visible();
                return;
            }
        }
        for index in (0..self.selected).rev() {
            if !self.is_cleanup_target_index(index) {
                self.selected = index;
                self.ensure_visible();
                return;
            }
        }
    }

    pub fn finish_cleanup_progress(&mut self) {
        self.cleanup_in_progress = false;
        self.cleanup_progress_total = 0;
        self.cleanup_progress_done = 0;
        self.cleanup_active_branch = None;
        self.cleanup_target_branches.clear();
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

    /// Prepare branch summary loading state for the selected branch.
    /// Returns a request payload for background fetch when needed.
    pub fn prepare_branch_summary(&mut self, repo_root: &Path) -> Option<BranchSummaryRequest> {
        let Some(branch) = self.selected_branch().cloned() else {
            self.branch_summary = None;
            return None;
        };

        let selected_changed = self
            .branch_summary
            .as_ref()
            .map(|summary| summary.branch_name.as_str())
            != Some(branch.name.as_str());

        if selected_changed {
            self.session_scroll_offset = 0;
        }

        if !selected_changed && self.branch_summary.is_some() {
            return None;
        }

        let mut summary = BranchSummary::new(&branch.name);
        if let Some(wt_path) = &branch.worktree_path {
            summary = summary.with_worktree_path(Some(PathBuf::from(wt_path)));
            summary.loading.stats = true;
        }
        summary.loading.commits = true;
        summary.loading.meta = true;

        if self.ai_enabled {
            if let Some(summary_data) = self.session_summary_cache.get(&branch.name) {
                summary = summary.with_session_summary(summary_data.clone());
            } else if self.session_summary_inflight.contains(&branch.name) {
                summary.loading.session_summary = true;
            }
        }

        self.branch_summary = Some(summary);

        Some(BranchSummaryRequest {
            branch: branch.name.clone(),
            repo_root: repo_root.to_path_buf(),
            branch_item: branch,
        })
    }

    pub fn apply_branch_summary_update(&mut self, update: BranchSummaryUpdate) {
        let Some(selected) = self.selected_branch() else {
            return;
        };
        if selected.name != update.branch {
            return;
        }

        let mut summary = update.summary;
        if let Some(current) = self.branch_summary.as_ref() {
            if current.branch_name == update.branch {
                summary.session_summary = current.session_summary.clone();
                summary.loading.session_summary = current.loading.session_summary;
                summary.errors.session_summary = current.errors.session_summary.clone();
            }
        }
        self.branch_summary = Some(summary);
    }

    /// Build branch summary data in background.
    pub fn build_branch_summary(repo_root: &Path, branch: &BranchItem) -> BranchSummary {
        let mut summary = BranchSummary::new(&branch.name);
        if let Some(wt_path) = &branch.worktree_path {
            summary = summary.with_worktree_path(Some(PathBuf::from(wt_path)));
        }

        let repo_path = branch
            .worktree_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_root.to_path_buf());

        match Repository::open(&repo_path) {
            Ok(repo) => {
                match repo.get_commit_log(5) {
                    Ok(commits) => {
                        summary = summary.with_commits(commits);
                    }
                    Err(e) => {
                        summary.errors.commits = Some(e.to_string());
                    }
                }

                if branch.worktree_path.is_some() {
                    match repo.get_diff_stats() {
                        Ok(mut stats) => {
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
            Err(e) => {
                summary.errors.commits = Some(e.to_string());
                if branch.worktree_path.is_some() {
                    summary.errors.stats = Some(e.to_string());
                }
            }
        }

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

        summary
    }

    pub fn session_summary_cached(&self, branch: &str) -> bool {
        self.session_summary_cache.get(branch).is_some()
    }

    pub fn set_session_identity(&mut self, branch: &str, tool_id: String, session_id: String) {
        if let Some(item) = self.branches.iter_mut().find(|b| b.name == branch) {
            item.last_tool_id = Some(tool_id);
            item.last_session_id = Some(session_id);
        }
    }

    pub fn update_session_scroll_bounds(&mut self, max_scroll: usize, page_size: usize) {
        self.session_scroll_max = max_scroll;
        self.session_scroll_page = page_size;
        if self.session_scroll_offset > max_scroll {
            self.session_scroll_offset = max_scroll;
        }
    }

    pub fn scroll_session_page_up(&mut self) {
        let page = self.session_scroll_page.max(1);
        self.session_scroll_offset = self.session_scroll_offset.saturating_sub(page);
    }

    pub fn scroll_session_page_down(&mut self) {
        let page = self.session_scroll_page.max(1);
        self.session_scroll_offset =
            (self.session_scroll_offset + page).min(self.session_scroll_max);
    }

    pub fn scroll_session_line_up(&mut self) {
        self.session_scroll_offset = self.session_scroll_offset.saturating_sub(1);
    }

    pub fn scroll_session_line_down(&mut self) {
        self.session_scroll_offset = (self.session_scroll_offset + 1).min(self.session_scroll_max);
    }

    pub fn session_summary(&self, branch: &str) -> Option<&gwt_core::ai::SessionSummary> {
        self.session_summary_cache.get(branch)
    }

    pub fn session_summary_warning(&self, branch: &str) -> Option<&String> {
        self.session_summary_warnings.get(branch)
    }

    pub fn session_summary_stale(
        &self,
        branch: &str,
        session_id: &str,
        mtime: std::time::SystemTime,
    ) -> bool {
        self.session_summary_cache
            .is_stale(branch, session_id, mtime)
    }

    pub fn clone_session_cache(&self) -> SessionSummaryCache {
        self.session_summary_cache.clone()
    }

    /// Cleanup session summary warnings for deleted branches
    pub fn cleanup_session_warnings(&mut self, remaining_branches: &HashSet<String>) {
        self.session_summary_warnings
            .retain(|name, _| remaining_branches.contains(name));
    }

    pub fn set_session_cache(&mut self, cache: SessionSummaryCache) {
        self.session_summary_cache = cache;
    }

    pub fn session_summary_inflight(&self, branch: &str) -> bool {
        self.session_summary_inflight.contains(branch)
    }

    pub fn clone_session_inflight(&self) -> HashSet<String> {
        self.session_summary_inflight.clone()
    }

    pub fn set_session_inflight(&mut self, inflight: HashSet<String>) {
        self.session_summary_inflight = inflight;
    }

    pub fn clone_session_warnings(&self) -> HashMap<String, String> {
        self.session_summary_warnings.clone()
    }

    pub fn set_session_warnings(&mut self, warnings: HashMap<String, String>) {
        self.session_summary_warnings = warnings;
    }

    pub fn clone_session_missing(&self) -> HashSet<String> {
        self.session_missing.clone()
    }

    pub fn set_session_missing(&mut self, missing: HashSet<String>) {
        self.session_missing = missing;
    }

    pub fn mark_session_summary_inflight(&mut self, branch: &str) {
        self.session_summary_inflight.insert(branch.to_string());
    }

    pub fn mark_session_missing(&mut self, branch: &str) {
        self.session_missing.insert(branch.to_string());
    }

    pub fn clear_session_missing(&mut self, branch: &str) {
        self.session_missing.remove(branch);
    }

    pub fn is_session_missing(&self, branch: &str) -> bool {
        self.session_missing.contains(branch)
    }

    pub fn apply_session_summary(
        &mut self,
        branch: &str,
        session_id: &str,
        summary: gwt_core::ai::SessionSummary,
        mtime: std::time::SystemTime,
    ) {
        self.session_summary_cache.set(
            branch.to_string(),
            session_id.to_string(),
            summary.clone(),
            mtime,
        );
        self.session_summary_inflight.remove(branch);
        self.session_summary_warnings.remove(branch);
        self.session_missing.remove(branch);
        if let Some(current) = self.branch_summary.as_mut() {
            if current.branch_name == branch {
                current.session_summary = Some(summary);
                current.loading.session_summary = false;
                current.errors.session_summary = None;
            }
        }
    }

    pub fn apply_session_warning(&mut self, branch: &str, warning: String) {
        self.session_summary_inflight.remove(branch);
        self.session_summary_warnings
            .insert(branch.to_string(), warning);
        if let Some(current) = self.branch_summary.as_mut() {
            if current.branch_name == branch {
                current.loading.session_summary = false;
            }
        }
    }

    pub fn apply_session_error(&mut self, branch: &str, error: String) {
        self.session_summary_inflight.remove(branch);
        self.session_summary_warnings.remove(branch);
        if let Some(current) = self.branch_summary.as_mut() {
            if current.branch_name == branch {
                if current.session_summary.is_none() {
                    current.errors.session_summary = Some(error);
                }
                current.loading.session_summary = false;
            }
        }
    }

    /// Clear branch summary (called when branches are reloaded)
    pub fn clear_branch_summary(&mut self) {
        self.branch_summary = None;
    }
}

/// Render branch list screen
/// Note: Header, Filter, Mode are rendered by app.rs view_boxed_header
/// This function only renders the main panels inside the content area.
pub fn render_branch_list(
    state: &mut BranchListState,
    frame: &mut Frame,
    area: Rect,
    status_message: Option<&str>,
    has_focus: bool,
) {
    // SPEC-1ea18899 US4: Changed from 3-pane to 2-pane layout (removed Details panel)
    // Use 'v' key to open GitView for detailed branch info
    let panel_height = crate::tui::components::SummaryPanel::height();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(panel_height), // Branch list
            Constraint::Min(6),               // Session panel
        ])
        .split(area);

    // Cache branch list area for mouse selection and update visible height
    state.update_list_area(chunks[0]);

    render_branches(state, frame, chunks[0], has_focus);
    // SPEC-1ea18899 US4: Only render Session panel (Details panel removed)
    state.clear_detail_links();
    render_session_panel(state, frame, chunks[1], status_message);
}

/// Build the unified status bar line for the bottom status bar (FR-093~FR-096).
pub fn build_status_bar_line(
    state: &BranchListState,
    status_message: Option<&str>,
) -> Line<'static> {
    let agents: Vec<_> = state.running_agents.values().cloned().collect();
    let summary = StatusBarSummary::from_agents(&agents);

    let mut spans = Vec::new();

    // Add "Agents: " prefix
    spans.push(Span::styled(
        "Agents: ",
        Style::default().fg(Color::DarkGray),
    ));

    if summary.has_agents() {
        // Add running count
        if summary.running_count > 0 {
            spans.push(Span::styled(
                format!("{} running", summary.running_count),
                Style::default().fg(Color::Green),
            ));
        }

        // Add separator if needed
        if summary.running_count > 0 && (summary.waiting_count > 0 || summary.stopped_count > 0) {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }

        // Add waiting count (FR-104c: highlighted in yellow)
        if summary.waiting_count > 0 {
            spans.push(Span::styled(
                format!("{} waiting", summary.waiting_count),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Add separator if needed
        if summary.waiting_count > 0 && summary.stopped_count > 0 {
            spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        }

        // Add stopped count
        if summary.stopped_count > 0 {
            spans.push(Span::styled(
                format!("{} stopped", summary.stopped_count),
                Style::default().fg(Color::Red),
            ));
        }
    } else {
        spans.push(Span::styled("none", Style::default().fg(Color::DarkGray)));
    }

    let selected_count = state.selected_branches.len();
    if selected_count > 0 {
        spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("Selected: {}", selected_count),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(message) = status_message {
        let style = if message.starts_with("Error") || message.starts_with("Failed") {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(message.to_string(), style));
    }

    Line::from(spans)
}

/// Render agent status bar (kept for compatibility in tests and previews).
fn render_status_bar(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let line = build_status_bar_line(state, None);
    frame.render_widget(Paragraph::new(line), area);
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
        .border_style(border_style)
        .title(" Branches ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .padding(Padding::new(
            BRANCH_LIST_PADDING_X,
            BRANCH_LIST_PADDING_X,
            0,
            0,
        ));
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
    let cleanup_active_branch = state.cleanup_active_branch();
    let cleanup_target_branches = &state.cleanup_target_branches;
    let mut items: Vec<ListItem> = state
        .visible_filtered_indices(visible_height)
        .iter()
        .enumerate()
        .map(|(i, index)| {
            let branch = &state.branches[*index];
            let running_agent = state.get_running_agent(&branch.name);
            let is_cleanup_active = cleanup_active_branch
                .map(|name| name == branch.name)
                .unwrap_or(false);
            // FR-013: Check if branch is a cleanup target
            let is_cleanup_target =
                state.cleanup_in_progress && cleanup_target_branches.contains(&branch.name);
            render_branch_row(
                branch,
                state.offset + i == state.selected,
                &state.selected_branches,
                spinner_frame,
                is_cleanup_active,
                is_cleanup_target,
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
/// FR-011f: Show spinner in safety icon while cleanup is running
/// FR-013: Show DarkGray background for cleanup target branches
/// FR-020~024: Running agent info displayed on right side (right-aligned)
#[allow(clippy::too_many_arguments)]
fn render_branch_row(
    branch: &BranchItem,
    is_selected: bool,
    selected_set: &HashSet<String>,
    spinner_frame: usize,
    cleanup_active: bool,
    is_cleanup_target: bool,
    running_agent: Option<&AgentPane>,
    width: u16,
) -> ListItem<'static> {
    // Only show selection icons when at least one branch is selected
    let show_selection = !selected_set.is_empty();
    let is_checked = selected_set.contains(&branch.name);
    let (safety_icon, safety_color) = if cleanup_active {
        let spinner_char = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
        (spinner_char.to_string(), Color::Yellow)
    } else {
        // FR-031b: Pass spinner_frame for pending safety check
        branch.safety_icon(Some(spinner_frame))
    };
    // FR-082/FR-083/FR-084/FR-085: Get branch name color based on worktree/gone status
    let branch_name_color = branch.branch_name_color();

    // Branch name (strip "remotes/" prefix for cleaner display)
    let display_name = if branch.branch_type == BranchType::Remote {
        let name = branch.remote_name.as_deref().unwrap_or(&branch.name);
        name.strip_prefix("remotes/").unwrap_or(name)
    } else {
        &branch.name
    };
    // SPEC-a70a1ece FR-101: Remove (current) label - branch shown in header instead
    let current_label = "";

    // Calculate left side width: optionally "[*] " + safety + " " + branch_name
    // FR-082: Worktree column removed, branch name color indicates status
    // selection_icon(3) + space(1) if showing selection, plus safety_icon + space(1) + name
    let selection_width = if show_selection { 3 } else { 0 }; // "◉ " or "◎ " (2 + 1)
    let left_width =
        selection_width + safety_icon.len() + 1 + display_name.width() + current_label.width();

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
                let should_show = (spinner_frame / 2).is_multiple_of(2);
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
            Span::styled(
                format!("{} ", status_icon),
                Style::default().fg(status_color),
            ),
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

    // FR-082: Worktree column removed, branch name color indicates status
    spans.extend([
        Span::styled(safety_icon, Style::default().fg(safety_color)),
        Span::raw(" "),
        Span::styled(
            display_name.to_string(),
            Style::default().fg(branch_name_color),
        ),
    ]);
    if branch.is_current {
        spans.push(Span::styled(
            current_label,
            Style::default().fg(Color::Green),
        ));
    }

    // Add padding and right side if there's agent info
    if !right_spans.is_empty() {
        spans.push(Span::raw(" ".repeat(padding)));
        spans.extend(right_spans);
    }

    // FR-018: Selected branch shown with cyan background
    // FR-013: Cleanup target branches shown with DarkGray background and text
    let style = if is_selected {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else if is_cleanup_target {
        Style::default().bg(Color::DarkGray).fg(Color::DarkGray)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

// SPEC-1ea18899 US4: render_summary_panels removed (use GitView for details)

fn panel_title_line(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {} ", label),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn session_panel_hint() -> Line<'static> {
    Line::from(Span::styled(
        " PgUp/PgDn/Wheel: Scroll ",
        Style::default().fg(Color::DarkGray),
    ))
    .right_aligned()
}

fn session_scroll_layout(inner: Rect, scrollable: bool) -> (Rect, Option<Rect>) {
    if !scrollable || inner.width <= 1 {
        return (inner, None);
    }

    let content = Rect {
        width: inner.width.saturating_sub(1),
        ..inner
    };
    let scrollbar = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y,
        width: 1,
        height: inner.height,
    };
    (content, Some(scrollbar))
}

fn session_scrollbar_content_length(total_lines: usize, viewport_len: usize) -> usize {
    if total_lines == 0 {
        return 1;
    }
    let viewport_len = viewport_len.max(1);
    total_lines.saturating_sub(viewport_len).saturating_add(1)
}

// SPEC-1ea18899 US4: build_summary_links, normalize_branch_name_for_url, render_details_panel removed

fn render_session_panel(
    state: &mut BranchListState,
    frame: &mut Frame,
    area: Rect,
    _status_message: Option<&str>,
) {
    let title = panel_title_line("Session");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(title)
        .title_bottom(session_panel_hint())
        .padding(Padding::new(PANEL_PADDING_X, PANEL_PADDING_X, 0, 0));
    let inner = block.inner(area);
    state.update_session_panel_area(area, inner);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Show active status line (cleanup/status progress) at top if present
    if let Some(status_line) = state.active_status_line() {
        lines.push(Line::from(Span::styled(
            status_line,
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    if state.is_loading {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", state.spinner_char()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                "Loading branch information...",
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else if let Some(branch) = state.selected_branch() {
        if !state.ai_enabled {
            lines.push(Line::from(Span::styled(
                "Configure AI in Profiles to enable session summary",
                Style::default().fg(Color::Yellow),
            )));
        } else if branch.last_session_id.is_none() || state.is_session_missing(&branch.name) {
            lines.push(Line::from(Span::styled(
                "No session",
                Style::default().fg(Color::DarkGray),
            )));
        } else if let Some(summary) = state.session_summary(&branch.name) {
            if let Some(warning) = state.session_summary_warning(&branch.name) {
                lines.push(Line::from(Span::styled(
                    warning.to_string(),
                    Style::default().fg(Color::Yellow),
                )));
                lines.push(Line::from(""));
            }
            let markdown = summary.markdown.clone();
            let task_overview = summary.task_overview.clone();
            let short_summary = summary.short_summary.clone();
            let bullet_points = summary.bullet_points.clone();
            if let Some(markdown) = markdown.as_ref() {
                lines = render_markdown_lines(markdown);
                if lines.is_empty() {
                    lines.push(Line::from(markdown.to_string()));
                }
            } else {
                lines.push(Line::from(Span::styled(
                    "Task:",
                    Style::default().fg(Color::Yellow),
                )));
                if let Some(task) = task_overview.as_ref() {
                    lines.push(Line::from(format!("  {}", task)));
                } else {
                    lines.push(Line::from(Span::styled(
                        "  (Not available)",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                lines.push(Line::from(Span::styled(
                    "Summary:",
                    Style::default().fg(Color::Yellow),
                )));
                if let Some(short) = short_summary.as_ref() {
                    lines.push(Line::from(format!("  {}", short)));
                } else {
                    lines.push(Line::from(Span::styled(
                        "  (Not available)",
                        Style::default().fg(Color::DarkGray),
                    )));
                }

                lines.push(Line::from(Span::styled(
                    "Highlights:",
                    Style::default().fg(Color::Yellow),
                )));
                if bullet_points.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  (No highlights)",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for bullet in bullet_points.iter() {
                        lines.push(Line::from(format!("  {}", bullet)));
                    }
                }
            }
        } else if state.session_summary_inflight(&branch.name) {
            lines.push(Line::from(Span::styled(
                format!("{} Generating session summary...", state.spinner_char()),
                Style::default().fg(Color::Yellow),
            )));
        } else if let Some(error) = state
            .branch_summary
            .as_ref()
            .and_then(|summary| summary.errors.session_summary.as_ref())
        {
            lines.push(Line::from(Span::styled(
                format!("(Failed to load: {})", error),
                Style::default().fg(Color::Red),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "Generating session summary...",
                Style::default().fg(Color::Yellow),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No branch selected",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let mut wrapped_lines = wrap_lines_by_char(lines.clone(), inner.width as usize);
    let mut total_lines = wrapped_lines.len();
    let scrollable = total_lines > inner.height as usize;
    let (content_area, scrollbar_area) = session_scroll_layout(inner, scrollable);
    if content_area.width != inner.width {
        wrapped_lines = wrap_lines_by_char(lines, content_area.width as usize);
        total_lines = wrapped_lines.len();
    }

    let viewport_len = content_area.height as usize;
    let max_scroll = total_lines.saturating_sub(viewport_len);
    state.update_session_scroll_bounds(max_scroll, viewport_len);

    let paragraph = Paragraph::new(wrapped_lines).scroll((state.session_scroll_offset as u16, 0));
    frame.render_widget(paragraph, content_area);

    if let Some(scrollbar_area) = scrollbar_area {
        let scrollbar_length = session_scrollbar_content_length(total_lines, viewport_len);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("^"))
            .end_symbol(Some("v"));
        let mut scrollbar_state = ScrollbarState::new(scrollbar_length)
            .position(state.session_scroll_offset)
            .viewport_content_length(viewport_len);
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

fn render_markdown_lines(markdown: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut buffer = String::new();
    let mut in_item = false;

    let options = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(markdown, options);

    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                flush_paragraph_lines(&mut lines, &mut buffer, false);
                buffer.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let text = buffer.trim();
                if !text.is_empty() {
                    lines.push(Line::from(Span::styled(
                        text.to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                buffer.clear();
            }
            Event::Start(Tag::Item) => {
                flush_paragraph_lines(&mut lines, &mut buffer, false);
                buffer.clear();
                in_item = true;
            }
            Event::End(TagEnd::Item) => {
                let text = buffer.trim();
                if !text.is_empty() {
                    push_bullet_lines(&mut lines, text);
                }
                buffer.clear();
                in_item = false;
            }
            Event::Start(Tag::Paragraph) => {
                if !in_item {
                    buffer.clear();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if !in_item {
                    flush_paragraph_lines(&mut lines, &mut buffer, false);
                }
            }
            Event::Text(text) => buffer.push_str(text.as_ref()),
            Event::Code(text) => buffer.push_str(text.as_ref()),
            Event::SoftBreak => buffer.push(' '),
            Event::HardBreak => buffer.push('\n'),
            _ => {}
        }
    }

    flush_paragraph_lines(&mut lines, &mut buffer, in_item);

    lines
}

fn wrap_lines_by_char(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in lines {
        wrapped.extend(wrap_line_chars(&line, width));
    }
    wrapped
}

fn wrap_line_chars(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    if line.spans.is_empty() {
        return vec![Line {
            spans: Vec::new(),
            style: line.style,
            alignment: line.alignment,
        }];
    }

    let mut wrapped = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;
    let mut buffer = String::new();
    let mut buffer_style = Style::default();
    let mut buffer_active = false;

    let flush_buffer = |spans: &mut Vec<Span<'static>>,
                        buffer: &mut String,
                        buffer_style: &Style,
                        buffer_active: &mut bool| {
        if *buffer_active && !buffer.is_empty() {
            spans.push(Span::styled(std::mem::take(buffer), *buffer_style));
        } else {
            buffer.clear();
        }
        *buffer_active = false;
    };

    let push_line =
        |wrapped: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>, line: &Line<'static>| {
            wrapped.push(Line {
                spans: std::mem::take(spans),
                style: line.style,
                alignment: line.alignment,
            });
        };

    for span in &line.spans {
        let span_style = span.style;
        for ch in span.content.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width > 0 && current_width + ch_width > width {
                flush_buffer(
                    &mut current_spans,
                    &mut buffer,
                    &buffer_style,
                    &mut buffer_active,
                );
                push_line(&mut wrapped, &mut current_spans, line);
                current_width = 0;
            }

            if !buffer_active || buffer_style != span_style {
                flush_buffer(
                    &mut current_spans,
                    &mut buffer,
                    &buffer_style,
                    &mut buffer_active,
                );
                buffer_style = span_style;
                buffer_active = true;
            }

            buffer.push(ch);
            current_width += ch_width;

            if current_width >= width {
                flush_buffer(
                    &mut current_spans,
                    &mut buffer,
                    &buffer_style,
                    &mut buffer_active,
                );
                push_line(&mut wrapped, &mut current_spans, line);
                current_width = 0;
            }
        }
    }

    flush_buffer(
        &mut current_spans,
        &mut buffer,
        &buffer_style,
        &mut buffer_active,
    );
    if !current_spans.is_empty() || wrapped.is_empty() {
        push_line(&mut wrapped, &mut current_spans, line);
    }

    wrapped
}

fn flush_paragraph_lines(lines: &mut Vec<Line<'static>>, buffer: &mut String, in_item: bool) {
    let text = buffer.trim();
    if text.is_empty() {
        return;
    }
    if in_item {
        push_bullet_lines(lines, text);
    } else {
        push_plain_lines(lines, text);
    }
    buffer.clear();
}

fn push_plain_lines(lines: &mut Vec<Line<'static>>, text: &str) {
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(trimmed.to_string()));
        }
    }
}

fn push_bullet_lines(lines: &mut Vec<Line<'static>>, text: &str) {
    let mut iter = text.split('\n');
    if let Some(first) = iter.next() {
        let trimmed = first.trim();
        if !trimmed.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("- ", Style::default().fg(Color::DarkGray)),
                Span::raw(trimmed.to_string()),
            ]));
        }
    }
    for rest in iter {
        let trimmed = rest.trim();
        if trimmed.is_empty() {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::raw(trimmed.to_string()),
        ]));
    }
}

/// Render footer line with keybindings (FR-004)
fn render_footer(frame: &mut Frame, area: Rect) {
    let keybinds = [
        ("r", "Refresh"),
        ("c", "Cleanup"),
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
    use gwt_core::git::CommitEntry;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::Path;
    use tempfile::tempdir;

    fn line_text(line: &Line<'_>) -> String {
        let mut text = String::new();
        for span in &line.spans {
            text.push_str(span.content.as_ref());
        }
        text
    }

    fn sample_branch(name: &str) -> BranchItem {
        BranchItem {
            name: name.to_string(),
            branch_type: BranchType::Local,
            is_current: false,
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
        }
    }

    fn sort_branch(name: &str) -> BranchItem {
        let mut item = sample_branch(name);
        item.has_worktree = false;
        item.worktree_status = WorktreeStatus::None;
        item.safe_to_cleanup = Some(true);
        item
    }

    #[test]
    fn test_view_mode_cycle() {
        assert_eq!(ViewMode::All.cycle(), ViewMode::Local);
        assert_eq!(ViewMode::Local.cycle(), ViewMode::Remote);
        assert_eq!(ViewMode::Remote.cycle(), ViewMode::All);
    }

    #[test]
    fn test_sort_default_type_order() {
        let branches = vec![
            sort_branch("feature/one"),
            sort_branch("bugfix/one"),
            sort_branch("hotfix/one"),
            sort_branch("release/one"),
            sort_branch("develop"),
            sort_branch("main"),
            sort_branch("chore/one"),
        ];
        let state = BranchListState::new().with_branches(branches);
        let names: Vec<_> = state
            .filtered_branches()
            .iter()
            .map(|branch| branch.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "main",
                "develop",
                "feature/one",
                "bugfix/one",
                "hotfix/one",
                "release/one",
                "chore/one",
            ]
        );
    }

    #[test]
    fn test_branch_name_type_ignores_suffix_match() {
        assert_eq!(
            get_branch_name_type("feature/main"),
            BranchNameType::Feature
        );
    }

    #[test]
    fn test_sort_current_branch_first() {
        let mut current = sort_branch("feature/current");
        current.is_current = true;
        let branches = vec![sort_branch("main"), current, sort_branch("develop")];
        let state = BranchListState::new().with_branches(branches);
        assert_eq!(state.filtered_branches()[0].name, "feature/current");
    }

    #[test]
    fn test_sort_mode_name_orders_by_name() {
        let branches = vec![sort_branch("feature/beta"), sort_branch("feature/alpha")];
        let mut state = BranchListState::new().with_branches(branches);
        state.set_sort_mode(BranchSortMode::Name);
        let names: Vec<_> = state
            .filtered_branches()
            .iter()
            .map(|branch| branch.name.as_str())
            .collect();
        assert_eq!(names, vec!["feature/alpha", "feature/beta"]);
    }

    #[test]
    fn test_sort_mode_updated_orders_by_timestamp() {
        let mut older = sort_branch("feature/old");
        older.last_commit_timestamp = Some(100);
        let mut newer = sort_branch("feature/new");
        newer.last_commit_timestamp = Some(200);
        let mut unknown = sort_branch("feature/unknown");
        unknown.last_commit_timestamp = None;

        let branches = vec![older, unknown, newer];
        let mut state = BranchListState::new().with_branches(branches);
        state.set_sort_mode(BranchSortMode::Updated);
        let names: Vec<_> = state
            .filtered_branches()
            .iter()
            .map(|branch| branch.name.as_str())
            .collect();
        assert_eq!(names, vec!["feature/new", "feature/old", "feature/unknown"]);
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
    fn test_wrap_lines_by_char_splits_long_line() {
        let lines = vec![Line::from("abcdef")];
        let wrapped = wrap_lines_by_char(lines, 3);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(line_text(&wrapped[0]), "abc");
        assert_eq!(line_text(&wrapped[1]), "def");
    }

    #[test]
    fn test_session_scroll_layout_no_scrollbar_when_not_scrollable() {
        let inner = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 5,
        };
        let (content, scrollbar) = session_scroll_layout(inner, false);
        assert_eq!(content, inner);
        assert!(scrollbar.is_none());
    }

    #[test]
    fn test_session_scroll_layout_scrollbar_when_scrollable() {
        let inner = Rect {
            x: 2,
            y: 3,
            width: 10,
            height: 5,
        };
        let (content, scrollbar) = session_scroll_layout(inner, true);
        assert_eq!(content.width, 9);
        assert_eq!(content.x, inner.x);
        assert_eq!(content.y, inner.y);
        assert_eq!(content.height, inner.height);
        let scrollbar = scrollbar.expect("scrollbar");
        assert_eq!(scrollbar.x, inner.x + inner.width - 1);
        assert_eq!(scrollbar.y, inner.y);
        assert_eq!(scrollbar.width, 1);
        assert_eq!(scrollbar.height, inner.height);
    }

    #[test]
    fn test_session_scroll_layout_no_scrollbar_when_narrow() {
        let inner = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 5,
        };
        let (content, scrollbar) = session_scroll_layout(inner, true);
        assert_eq!(content, inner);
        assert!(scrollbar.is_none());
    }

    #[test]
    fn test_session_scrollbar_content_length_from_total_and_viewport() {
        assert_eq!(session_scrollbar_content_length(10, 3), 8);
        assert_eq!(session_scrollbar_content_length(5, 5), 1);
        assert_eq!(session_scrollbar_content_length(0, 5), 1);
    }

    #[test]
    fn test_session_panel_contains_point() {
        let mut state = BranchListState::new();
        let area = Rect::new(0, 0, 20, 5);
        let inner = Rect::new(2, 1, 16, 3);
        state.update_session_panel_area(area, inner);

        assert!(state.session_panel_contains(2, 1));
        assert!(state.session_panel_contains(17, 3));
        assert!(!state.session_panel_contains(1, 1));
        assert!(!state.session_panel_contains(18, 1));
        assert!(!state.session_panel_contains(2, 4));
    }

    #[test]
    fn test_session_panel_contains_point_with_empty_inner() {
        let mut state = BranchListState::new();
        let area = Rect::new(0, 0, 2, 2);
        let inner = Rect::new(0, 0, 0, 0);
        state.update_session_panel_area(area, inner);

        assert!(!state.session_panel_contains(0, 0));
    }

    #[test]
    fn test_session_scroll_line_clamps_to_bounds() {
        let mut state = BranchListState::new();
        state.update_session_scroll_bounds(3, 5);
        state.session_scroll_offset = 2;

        state.scroll_session_line_down();
        assert_eq!(state.session_scroll_offset, 3);
        state.scroll_session_line_down();
        assert_eq!(state.session_scroll_offset, 3);
        state.scroll_session_line_up();
        assert_eq!(state.session_scroll_offset, 2);
        state.scroll_session_line_up();
        state.scroll_session_line_up();
        state.scroll_session_line_up();
        assert_eq!(state.session_scroll_offset, 0);
    }

    #[test]
    fn test_branch_name_color_by_worktree_status() {
        let mut branch = sample_branch("feature/color");

        branch.worktree_status = WorktreeStatus::Active;
        branch.is_gone = false;
        assert_eq!(branch.branch_name_color(), Color::White);

        branch.worktree_status = WorktreeStatus::None;
        assert_eq!(branch.branch_name_color(), Color::DarkGray);

        branch.worktree_status = WorktreeStatus::Inaccessible;
        assert_eq!(branch.branch_name_color(), Color::Red);

        branch.is_gone = true;
        branch.worktree_status = WorktreeStatus::None;
        assert_eq!(branch.branch_name_color(), Color::Red);
    }

    #[test]
    fn test_current_branch_label_not_displayed() {
        // SPEC-a70a1ece FR-101: (current) label removed - branch shown in header instead
        let mut branch = sample_branch("main");
        branch.is_current = true;

        let other = sample_branch("feature/other");
        let mut state = BranchListState::new().with_branches(vec![branch, other]);
        state.selected = 1;

        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branches(&state, f, area, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let label = "(current)";
        let label_chars: Vec<char> = label.chars().collect();
        let width = 40u16;
        let height = 5u16;

        // Verify (current) label does NOT appear anywhere
        for y in 0..height {
            for x in 0..=width.saturating_sub(label_chars.len() as u16) {
                let matches = label_chars
                    .iter()
                    .enumerate()
                    .all(|(offset, ch)| buffer[(x + offset as u16, y)].symbol().starts_with(*ch));
                assert!(!matches, "(current) label should NOT appear in branch row");
            }
        }
    }

    #[test]
    fn test_apply_worktree_created_updates_branch_and_preserves_selection() {
        let temp = tempdir().expect("tempdir");
        let expected_path = temp.path().display().to_string();

        let mut branch_a = sample_branch("feature/a");
        branch_a.has_worktree = false;
        branch_a.worktree_path = None;
        branch_a.worktree_status = WorktreeStatus::None;

        let mut branch_b = sample_branch("feature/b");
        branch_b.worktree_path = Some("/path".to_string());

        let mut state = BranchListState::new().with_branches(vec![branch_a, branch_b]);
        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("feature/b")
        );

        let updated = state.apply_worktree_created("feature/a", temp.path());
        assert!(updated);

        let updated_branch = state
            .branches
            .iter()
            .find(|branch| branch.name == "feature/a")
            .expect("branch exists");
        assert!(updated_branch.has_worktree);
        assert_eq!(updated_branch.worktree_status, WorktreeStatus::Active);
        assert_eq!(
            updated_branch.worktree_path.as_deref(),
            Some(expected_path.as_str())
        );
        assert_eq!(state.stats.worktree_count, 2);
        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("feature/b")
        );
    }

    #[test]
    fn test_prepare_branch_summary_resets_loading() {
        let branches = vec![sample_branch("feature/one"), sample_branch("feature/two")];
        let mut state = BranchListState::new().with_branches(branches);

        let mut existing = BranchSummary::new("feature/one");
        existing.commits = vec![CommitEntry {
            hash: "abc1234".to_string(),
            message: "feat: existing".to_string(),
        }];
        state.branch_summary = Some(existing);

        state.selected = 1;
        let request = state.prepare_branch_summary(Path::new("/repo"));
        assert!(request.is_some());

        let summary = state.branch_summary.as_ref().expect("summary");
        assert_eq!(summary.branch_name, "feature/two");
        assert!(summary.loading.commits);
        assert!(summary.loading.meta);
        assert!(summary.loading.stats);
        assert!(summary.commits.is_empty());
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
    fn test_cleanup_target_branch_is_skipped_by_cursor() {
        let branches = vec![
            sample_branch("branch-a"),
            sample_branch("branch-b"),
            sample_branch("branch-c"),
        ];
        let mut state = BranchListState::new().with_branches(branches);
        state.selected = 1;
        state.start_cleanup_progress(3);
        state.set_cleanup_target_branches(&["branch-b".to_string()]);

        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("branch-c")
        );

        state.selected = 0;
        state.select_next();
        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("branch-c")
        );

        state.select_prev();
        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("branch-a")
        );
    }

    #[test]
    fn test_select_index_ignores_cleanup_target_branch() {
        let branches = vec![
            sample_branch("branch-a"),
            sample_branch("branch-b"),
            sample_branch("branch-c"),
        ];
        let mut state = BranchListState::new().with_branches(branches);
        state.start_cleanup_progress(3);
        state.set_cleanup_target_branches(&["branch-b".to_string()]);

        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("branch-a")
        );

        let cleanup_index = state
            .filtered_indices
            .iter()
            .position(|&idx| state.branches[idx].name == "branch-b")
            .expect("cleanup branch index");
        assert!(!state.select_index(cleanup_index));
        assert_eq!(
            state.selected_branch().map(|branch| branch.name.as_str()),
            Some("branch-a")
        );
    }

    #[test]
    fn test_mouse_position_selects_visible_row() {
        let branches = vec![
            sample_branch("feature/one"),
            sample_branch("feature/two"),
            sample_branch("feature/three"),
        ];
        let mut state = BranchListState::new().with_branches(branches);
        state.update_list_area(Rect::new(0, 0, 20, 5)); // inner height = 3

        let index = state
            .selection_index_from_point(2, 2) // inner x=2,y=1 -> row 1
            .expect("index");
        assert_eq!(index, 1);
        assert!(state.select_index(index));
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn test_mouse_position_respects_offset_and_bounds() {
        let branches = vec![
            sample_branch("feature/one"),
            sample_branch("feature/two"),
            sample_branch("feature/three"),
            sample_branch("feature/four"),
        ];
        let mut state = BranchListState::new().with_branches(branches);
        state.update_list_area(Rect::new(2, 3, 20, 5)); // inner y=4..6
        state.offset = 1;

        let index = state
            .selection_index_from_point(4, 4) // inner top row
            .expect("index");
        assert_eq!(index, 1);

        assert!(state.selection_index_from_point(1, 1).is_none());
        assert!(state.selection_index_from_point(25, 10).is_none());
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
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
        assert_eq!(buffer[(1, 1)].symbol(), " ");
    }

    #[test]
    fn test_cleanup_active_branch_shows_spinner_in_safety_icon() {
        let branches = vec![BranchItem {
            name: "cleanupbranch".to_string(),
            branch_type: BranchType::Local,
            is_current: false,
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
        }];

        let mut state = BranchListState::new().with_branches(branches);
        state.start_cleanup_progress(1);
        state.set_cleanup_active_branch(Some("cleanupbranch".to_string()));
        state.spinner_frame = 0; // '|' frame

        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branches(&state, f, area, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let mut found = false;
        for y in 0..5 {
            let line: String = (0..30).map(|x| buffer[(x, y)].symbol()).collect();
            if line.contains("| cleanupbranch") {
                found = true;
                break;
            }
        }
        assert!(found, "cleanup spinner should appear in safety icon column");
    }

    /// FR-013: Cleanup target branches should have DarkGray background
    #[test]
    fn test_cleanup_target_branch_has_gray_background() {
        let branches = vec![
            BranchItem {
                name: "normal-branch".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
            BranchItem {
                name: "cleanup-target".to_string(),
                branch_type: BranchType::Local,
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                worktree_status: WorktreeStatus::None,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        state.start_cleanup_progress(1);
        state.set_cleanup_target_branches(&["cleanup-target".to_string()]);

        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branches(&state, f, area, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();

        // Find the row containing cleanup-target and check background/foreground color
        let mut cleanup_row_has_gray_bg = false;
        let mut cleanup_row_has_gray_fg = false;
        let mut normal_row_has_no_gray_bg = true;

        for y in 0..5 {
            let line: String = (0..40).map(|x| buffer[(x, y)].symbol()).collect();
            if line.contains("cleanup-target") {
                // Check that at least one cell has DarkGray background and foreground
                for x in 0..40 {
                    if buffer[(x, y)].bg == Color::DarkGray {
                        cleanup_row_has_gray_bg = true;
                    }
                    if buffer[(x, y)].fg == Color::DarkGray {
                        cleanup_row_has_gray_fg = true;
                    }
                }
            } else if line.contains("normal-branch") {
                // Check that normal branch does NOT have gray background
                for x in 0..40 {
                    if buffer[(x, y)].bg == Color::DarkGray {
                        normal_row_has_no_gray_bg = false;
                        break;
                    }
                }
            }
        }

        assert!(
            cleanup_row_has_gray_bg,
            "FR-013: Cleanup target branch should have DarkGray background"
        );
        assert!(
            cleanup_row_has_gray_fg,
            "FR-013: Cleanup target branch should have DarkGray text color"
        );
        assert!(
            normal_row_has_no_gray_bg,
            "Normal branch should not have DarkGray background"
        );
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
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
    fn test_cleanup_progress_line_renders() {
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
        }];

        let mut state = BranchListState::new().with_branches(branches);
        state.start_cleanup_progress(3);
        state.spinner_frame = 0;

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        terminal
            .draw(|f| {
                let area = f.area();
                render_branch_list(&mut state, f, area, None, true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let mut found = false;
        for y in 0..20 {
            let line: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if line.contains("Cleanup: Running") {
                found = true;
                break;
            }
        }
        assert!(found, "Cleanup progress line should appear in the panel");
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
    fn test_status_message_not_duplicated_in_panels() {
        let branches = vec![sample_branch("feature/status")];
        let mut state = BranchListState::new().with_branches(branches);
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("terminal init");

        let status = "STATUS_UNIQUE_123";
        terminal
            .draw(|f| {
                let area = f.area();
                render_branch_list(&mut state, f, area, Some(status), true);
            })
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let mut found = false;
        for y in 0..20 {
            let line: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if line.contains(status) {
                found = true;
                break;
            }
        }
        assert!(
            !found,
            "Global status message should not be rendered inside Details/Session panels"
        );
    }

    #[test]
    fn test_panel_title_line_is_label_only_and_consistent() {
        let title = panel_title_line("Details");
        let text = line_text(&title);
        assert_eq!(text, " Details ");
        assert_eq!(title.spans.len(), 1);

        let span = &title.spans[0];
        assert_eq!(span.style.fg, Some(Color::Cyan));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);

        // Default is Local, so only local branches are shown
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "main");

        state.set_view_mode(ViewMode::All);
        assert_eq!(state.filtered_branches().len(), 2);

        state.set_view_mode(ViewMode::Remote);
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "remotes/origin/main");
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
    fn test_apply_pr_info_updates_filter_results() {
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        state.set_filter("cool".to_string());
        assert_eq!(state.filtered_branches().len(), 0);

        let mut info = HashMap::new();
        info.insert(
            "feature/one".to_string(),
            PrInfo {
                title: "Cool PR".to_string(),
                number: 123,
                url: Some("https://github.com/example/repo/pull/123".to_string()),
                state: "OPEN".to_string(),
            },
        );
        state.apply_pr_info(&info);

        let filtered = state.filtered_branches();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "feature/one");
        assert_eq!(filtered[0].pr_state.as_deref(), Some("OPEN"));
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
            last_tool_id: None,
            last_session_id: None,
            is_selected: false,
            pr_title: None,
            pr_number: None,
            pr_url: None,
            pr_state: None,
            is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
                last_tool_id: None,
                last_session_id: None,
                is_selected: false,
                pr_title: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
                is_gone: false,
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
