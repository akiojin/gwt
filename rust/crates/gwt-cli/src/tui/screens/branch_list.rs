//! Branch List Screen - TypeScript版完全互換

#![allow(dead_code)]

use gwt_core::git::{Branch, DivergenceStatus};
use gwt_core::worktree::Worktree;
use ratatui::{prelude::*, widgets::*};
use std::collections::HashSet;
use std::time::Instant;

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
    Safe,
    Uncommitted,
    Unpushed,
    Unmerged,
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
    pub is_unmerged: bool,
    pub last_commit_timestamp: Option<i64>,
    pub last_tool_usage: Option<String>,
    pub is_selected: bool,
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
            wt.branch.as_ref().map(|b| b == &branch.name).unwrap_or(false)
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

        Self {
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
            is_unmerged: false,
            last_commit_timestamp: None,
            last_tool_usage: None,
            is_selected: false,
        }
    }

    /// Get safety icon and color
    pub fn safety_icon(&self) -> (&'static str, Color) {
        if self.branch_type == BranchType::Remote {
            return (" ", Color::Reset);
        }
        if self.has_changes {
            return ("!", Color::Red);
        }
        if self.has_unpushed {
            return ("!", Color::Yellow);
        }
        if self.is_unmerged {
            return ("*", Color::Yellow);
        }
        if self.safe_to_cleanup == Some(true) {
            return ("o", Color::Green);
        }
        ("!", Color::Red)
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

/// Branch list state
#[derive(Debug, Default)]
pub struct BranchListState {
    pub branches: Vec<BranchItem>,
    pub selected: usize,
    pub offset: usize,
    pub filter: String,
    pub filter_mode: bool,
    pub view_mode: ViewMode,
    pub selected_branches: HashSet<String>,
    pub stats: Statistics,
    pub last_updated: Option<Instant>,
    pub is_loading: bool,
    pub error: Option<String>,
    pub version: Option<String>,
    pub working_directory: Option<String>,
    pub active_profile: Option<String>,
}

impl BranchListState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_branches(mut self, branches: Vec<BranchItem>) -> Self {
        // Calculate statistics
        self.stats = Statistics {
            local_count: branches.iter().filter(|b| b.branch_type == BranchType::Local).count(),
            remote_count: branches.iter().filter(|b| b.branch_type == BranchType::Remote || b.has_remote_counterpart).count(),
            worktree_count: branches.iter().filter(|b| b.has_worktree).count(),
            changes_count: branches.iter().filter(|b| b.has_changes).count(),
        };
        self.branches = branches;
        self.last_updated = Some(Instant::now());
        self
    }

    /// Get filtered branches based on view mode and filter
    pub fn filtered_branches(&self) -> Vec<&BranchItem> {
        let mut result: Vec<&BranchItem> = self.branches.iter().collect();

        // Apply view mode filter
        result = match self.view_mode {
            ViewMode::All => result,
            ViewMode::Local => result.into_iter()
                .filter(|b| b.branch_type == BranchType::Local)
                .collect(),
            ViewMode::Remote => result.into_iter()
                .filter(|b| b.branch_type == BranchType::Remote || b.has_remote_counterpart)
                .collect(),
        };

        // Apply text filter
        if !self.filter.is_empty() {
            let filter_lower = self.filter.to_lowercase();
            result = result.into_iter()
                .filter(|b| b.name.to_lowercase().contains(&filter_lower))
                .collect();
        }

        result
    }

    /// Cycle view mode
    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.cycle();
        self.selected = 0;
        self.offset = 0;
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
    }

    /// Add char to filter
    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
        self.offset = 0;
    }

    /// Remove char from filter
    pub fn filter_pop(&mut self) {
        self.filter.pop();
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
        let filtered = self.filtered_branches();
        if !filtered.is_empty() && self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        let filtered = self.filtered_branches();
        if !filtered.is_empty() && self.selected < filtered.len() - 1 {
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
        let filtered = self.filtered_branches();
        if !filtered.is_empty() {
            self.selected = (self.selected + page_size).min(filtered.len() - 1);
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
        let filtered = self.filtered_branches();
        if !filtered.is_empty() {
            self.selected = filtered.len() - 1;
        }
        self.ensure_visible();
    }

    /// Ensure selected item is visible
    fn ensure_visible(&mut self) {
        let visible_window = 15;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + visible_window {
            self.offset = self.selected.saturating_sub(visible_window - 1);
        }
    }

    /// Get currently selected branch
    pub fn selected_branch(&self) -> Option<&BranchItem> {
        let filtered = self.filtered_branches();
        filtered.get(self.selected).copied()
    }

    /// Update filter and reset selection
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
        self.offset = 0;
    }

    /// Get relative time string
    fn format_relative_time(&self) -> String {
        if let Some(updated) = self.last_updated {
            let elapsed = updated.elapsed();
            let secs = elapsed.as_secs();
            if secs < 60 {
                format!("{}s ago", secs)
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else {
                format!("{}h ago", secs / 3600)
            }
        } else {
            String::new()
        }
    }
}

/// Render branch list screen
pub fn render_branch_list(
    state: &BranchListState,
    frame: &mut Frame,
    area: Rect,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Filter line
            Constraint::Length(1), // Stats line
            Constraint::Length(1), // Legend line
            Constraint::Min(3),    // Branch list
            Constraint::Length(1), // Worktree path
        ])
        .split(area);

    render_filter_line(state, frame, chunks[0]);
    render_stats_line(state, frame, chunks[1]);
    render_legend_line(frame, chunks[2]);
    render_branches(state, frame, chunks[3]);
    render_worktree_path(state, frame, chunks[4]);
}

/// Render filter line
fn render_filter_line(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_branches();
    let total = state.branches.len();

    let mut spans = vec![
        Span::styled("Filter(f): ", Style::default().fg(Color::DarkGray)),
    ];

    if state.filter_mode {
        if state.filter.is_empty() {
            spans.push(Span::styled("Type to search...", Style::default().fg(Color::DarkGray)));
        } else {
            spans.push(Span::raw(&state.filter));
        }
        spans.push(Span::styled("|", Style::default().fg(Color::White)));
    } else {
        spans.push(Span::styled(
            if state.filter.is_empty() { "(press f to filter)" } else { &state.filter },
            Style::default().fg(Color::DarkGray),
        ));
    }

    if !state.filter.is_empty() {
        spans.push(Span::styled(
            format!(" (Showing {} of {})", filtered.len(), total),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render stats line
fn render_stats_line(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let mut spans = vec![
        Span::styled("Mode(tab): ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.view_mode.label(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled("Local: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.stats.local_count.to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled("Remote: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.stats.remote_count.to_string(),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled("Worktrees: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.stats.worktree_count.to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled("Changes: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.stats.changes_count.to_string(),
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
    ];

    let relative_time = state.format_relative_time();
    if !relative_time.is_empty() {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled("Updated: ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(relative_time, Style::default().fg(Color::DarkGray)));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render legend line
fn render_legend_line(frame: &mut Frame, area: Rect) {
    let spans = vec![
        Span::styled("Legend: ", Style::default().fg(Color::DarkGray)),
        Span::styled("o", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(" Safe", Style::default().fg(Color::Green)),
        Span::styled("  ", Style::default()),
        Span::styled("!", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled(" Uncommitted", Style::default().fg(Color::Red)),
        Span::styled("  ", Style::default()),
        Span::styled("!", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(" Unpushed", Style::default().fg(Color::Yellow)),
        Span::styled("  ", Style::default()),
        Span::styled("*", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(" Unmerged", Style::default().fg(Color::Yellow)),
    ];

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render branches list
fn render_branches(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_branches();

    if filtered.is_empty() {
        let text = if state.is_loading {
            "Loading branches..."
        } else if state.filter.is_empty() {
            "No branches found"
        } else {
            "No branches match your filter"
        };
        let paragraph = Paragraph::new(text)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(paragraph, area);
        return;
    }

    let visible_height = area.height as usize;
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .skip(state.offset)
        .take(visible_height)
        .map(|(i, branch)| render_branch_row(branch, i == state.selected, &state.selected_branches))
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);

    // Scrollbar
    if filtered.len() > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("^"))
            .end_symbol(Some("v"));
        let mut scrollbar_state = ScrollbarState::new(filtered.len())
            .position(state.selected);
        frame.render_stateful_widget(
            scrollbar,
            area,
            &mut scrollbar_state,
        );
    }
}

/// Render a single branch row
fn render_branch_row(branch: &BranchItem, is_selected: bool, selected_set: &HashSet<String>) -> ListItem<'static> {
    let is_checked = selected_set.contains(&branch.name);
    let selection_icon = if is_checked { "[*]" } else { "[ ]" };
    let (worktree_icon, worktree_color) = branch.worktree_icon();
    let (safety_icon, safety_color) = branch.safety_icon();

    let mut spans = vec![
        Span::styled(
            selection_icon,
            if is_checked && (branch.has_changes || branch.has_unpushed) {
                Style::default().fg(Color::Red)
            } else {
                Style::default()
            },
        ),
        Span::raw(" "),
        Span::styled(worktree_icon, Style::default().fg(worktree_color)),
        Span::raw(" "),
        Span::styled(safety_icon, Style::default().fg(safety_color)),
        Span::raw(" "),
    ];

    // Branch name
    let display_name = if branch.branch_type == BranchType::Remote {
        branch.remote_name.as_deref().unwrap_or(&branch.name)
    } else {
        &branch.name
    };
    spans.push(Span::raw(display_name.to_string()));

    // Tool usage (if available)
    if let Some(tool) = &branch.last_tool_usage {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(tool.clone(), Style::default().fg(Color::Cyan)));
    }

    let style = if is_selected {
        Style::default().bg(Color::Blue).fg(Color::White)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

/// Render worktree path line
fn render_worktree_path(state: &BranchListState, frame: &mut Frame, area: Rect) {
    let path = if let Some(branch) = state.selected_branch() {
        branch.worktree_path.clone().unwrap_or_else(|| "(none)".to_string())
    } else {
        "(none)".to_string()
    };

    let spans = vec![
        Span::styled("Worktree: ", Style::default().fg(Color::DarkGray)),
        Span::styled(path, Style::default().fg(Color::DarkGray)),
    ];

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_mode_cycle() {
        assert_eq!(ViewMode::All.cycle(), ViewMode::Local);
        assert_eq!(ViewMode::Local.cycle(), ViewMode::Remote);
        assert_eq!(ViewMode::Remote.cycle(), ViewMode::All);
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
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
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
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
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
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
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
                is_unmerged: false,
                last_commit_timestamp: None,
                last_tool_usage: None,
                is_selected: false,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);

        assert_eq!(state.filtered_branches().len(), 2);

        state.view_mode = ViewMode::Local;
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "main");

        state.view_mode = ViewMode::Remote;
        assert_eq!(state.filtered_branches().len(), 2); // main has remote counterpart
    }
}
