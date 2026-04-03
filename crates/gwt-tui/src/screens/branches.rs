//! Branches management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

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
    #[default]
    All,
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
}

/// Number of detail sections in the branch detail view.
const DETAIL_SECTION_COUNT: usize = 4;

/// Labels for the detail sections.
const DETAIL_SECTION_LABELS: [&str; DETAIL_SECTION_COUNT] =
    ["Overview", "SPECs", "Git", "Sessions"];

/// Action labels in the Actions detail section.
const ACTION_LABELS: [&str; 3] = ["Launch Agent", "Open Shell", "Delete Worktree"];

/// State for the branches screen.
#[derive(Debug, Clone, Default)]
pub struct BranchesState {
    pub(crate) branches: Vec<BranchItem>,
    pub(crate) selected: usize,
    pub(crate) sort_mode: SortMode,
    pub(crate) view_mode: ViewMode,
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    pub(crate) detail_view: bool,
    /// Active detail section: 0=Overview, 1=SPECs, 2=Git, 3=Sessions.
    pub(crate) detail_section: usize,
    /// Whether the action modal overlay is visible.
    pub(crate) action_modal_visible: bool,
    /// Selected action index within the action modal.
    pub(crate) action_modal_selected: usize,
    /// Flag: caller should open agent selection.
    pub(crate) pending_launch_agent: bool,
    /// Flag: caller should spawn shell in worktree cwd.
    pub(crate) pending_open_shell: bool,
    /// Flag: caller should show worktree delete confirmation.
    pub(crate) pending_delete_worktree: bool,
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
            SortMode::Default => {} // insertion order
            // Date has no dedicated field yet; fall back to alphabetical like Name.
            SortMode::Name | SortMode::Date => result.sort_by(|a, b| a.name.cmp(&b.name)),
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
    /// Open the action modal overlay.
    OpenActionModal,
    /// Close the action modal overlay.
    CloseActionModal,
    /// Move up within the action modal.
    ActionModalUp,
    /// Move down within the action modal.
    ActionModalDown,
    /// Select the current action in the modal.
    ActionModalSelect,
    /// Launch agent action.
    LaunchAgent,
    /// Open shell action.
    OpenShell,
    /// Delete worktree action.
    DeleteWorktree,
}

/// Update branches state in response to a message.
pub fn update(state: &mut BranchesState, msg: BranchesMessage) {
    match msg {
        BranchesMessage::MoveUp => {
            let len = state.filtered_branches().len();
            super::move_up(&mut state.selected, len);
        }
        BranchesMessage::MoveDown => {
            let len = state.filtered_branches().len();
            super::move_down(&mut state.selected, len);
        }
        BranchesMessage::Select => {
            if !state.filtered_branches().is_empty() {
                state.detail_view = !state.detail_view;
            }
        }
        BranchesMessage::ToggleSort => {
            state.sort_mode = state.sort_mode.next();
        }
        BranchesMessage::ToggleView => {
            state.view_mode = state.view_mode.next();
            state.clamp_selected();
        }
        BranchesMessage::SearchStart => {
            state.search_active = true;
        }
        BranchesMessage::SearchInput(ch) => {
            if state.search_active {
                state.search_query.push(ch);
                state.clamp_selected();
            }
        }
        BranchesMessage::SearchBackspace => {
            if state.search_active {
                state.search_query.pop();
                state.clamp_selected();
            }
        }
        BranchesMessage::SearchClear => {
            state.search_query.clear();
            state.search_active = false;
            state.clamp_selected();
        }
        BranchesMessage::Refresh => {
            // Signal to reload branches — handled by caller
        }
        BranchesMessage::SetBranches(branches) => {
            state.branches = branches;
            state.clamp_selected();
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
        BranchesMessage::OpenActionModal => {
            if !state.filtered_branches().is_empty() {
                state.action_modal_visible = true;
                state.action_modal_selected = 0;
            }
        }
        BranchesMessage::CloseActionModal => {
            state.action_modal_visible = false;
        }
        BranchesMessage::ActionModalUp => {
            super::move_up(&mut state.action_modal_selected, ACTION_LABELS.len());
        }
        BranchesMessage::ActionModalDown => {
            super::move_down(&mut state.action_modal_selected, ACTION_LABELS.len());
        }
        BranchesMessage::ActionModalSelect => {
            let selected = state.action_modal_selected;
            state.action_modal_visible = false;
            match selected {
                0 => state.pending_launch_agent = true,
                1 => state.pending_open_shell = true,
                _ => state.pending_delete_worktree = true,
            }
        }
        BranchesMessage::LaunchAgent => {
            state.pending_launch_agent = true;
        }
        BranchesMessage::OpenShell => {
            state.pending_open_shell = true;
        }
        BranchesMessage::DeleteWorktree => {
            state.pending_delete_worktree = true;
        }
    }
}

/// Which sub-pane of the branches screen is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchesFocus {
    List,
    Detail,
    None,
}

/// Render the branches screen with split layout: top = list, bottom = detail.
pub fn render(state: &BranchesState, frame: &mut Frame, area: Rect, focus: BranchesFocus) {
    // Split vertically: top 50% for list, bottom 50% for detail
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Top half: header + branch list
    let list_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header: view/sort/search (no border)
            Constraint::Min(0),    // Branch list
        ])
        .split(main_chunks[0]);

    render_header(state, frame, list_chunks[0]);
    render_branch_list(state, frame, list_chunks[1], focus == BranchesFocus::List);

    // Bottom half: branch detail
    render_branch_detail(state, frame, main_chunks[1], focus == BranchesFocus::Detail);
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
            Style::default().fg(Color::White),
        ),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" Sort: {} ", state.sort_mode.label()),
            Style::default().fg(Color::White),
        ),
        Span::styled(search_display, Style::default().fg(Color::Yellow)),
    ]);

    let paragraph = Paragraph::new(line)
        .style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

/// Render the branch list grouped by category.
fn render_branch_list(state: &BranchesState, frame: &mut Frame, area: Rect, is_focused: bool) {
    let filtered = state.filtered_branches();

    if filtered.is_empty() {
        super::render_empty_list(frame, area, !state.branches.is_empty(), "branches");
        return;
    }

    // Build items with category headers, tracking visual index offset
    let mut items: Vec<ListItem> = Vec::new();
    let mut current_category: Option<BranchCategory> = None;
    let mut headers_before_selected: usize = 0;

    for (idx, branch) in filtered.iter().enumerate() {
        if current_category != Some(branch.category) {
            current_category = Some(branch.category);
            if idx <= state.selected {
                headers_before_selected += 1;
            }
            let header = Line::from(Span::styled(
                format!("── {} ──", branch.category.label()),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            items.push(ListItem::new(header));
        }

        let head_indicator = if branch.is_head { "* " } else { "  " };
        let locality = if branch.is_local { "L" } else { "R" };

        let line = Line::from(vec![
            Span::styled(head_indicator, Style::default().fg(Color::Green)),
            Span::styled(format!("[{}] ", locality), Style::default().fg(Color::Cyan)),
            Span::styled(&branch.name, Style::default().fg(Color::White)),
        ]);
        items.push(ListItem::new(line));
    }

    // Visual index = data index + number of headers inserted before it
    let visual_selected = state.selected + headers_before_selected;

    let border_color = if is_focused { Color::Cyan } else { Color::Gray };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(border_color));
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(visual_selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render the branch detail panel (bottom half).
fn render_branch_detail(state: &BranchesState, frame: &mut Frame, area: Rect, is_focused: bool) {
    let title = super::build_tab_title(&DETAIL_SECTION_LABELS, state.detail_section);
    let border_color = if is_focused { Color::Cyan } else { Color::Gray };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Section content
    match state.detail_section {
        0 => render_detail_overview(state, frame, inner),
        1 => render_detail_specs(state, frame, inner),
        2 => render_detail_git_status(state, frame, inner),
        3 => render_detail_sessions(frame, inner),
        _ => {}
    }

    // Action modal overlay (rendered on top of detail)
    if state.action_modal_visible {
        render_action_modal(state, frame, area);
    }
}

/// Overview section: branch name, HEAD indicator, category.
fn render_detail_overview(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let content = match state.selected_branch() {
        Some(branch) => {
            let head = if branch.is_head { " (HEAD)" } else { "" };
            let locality = if branch.is_local { "Local" } else { "Remote" };
            format!(
                " Branch: {}{}\n Category: {}\n Type: {}",
                branch.name,
                head,
                branch.category.label(),
                locality,
            )
        }
        None => " No branch selected".to_string(),
    };

    let block = Block::default().borders(Borders::ALL).title("Overview");
    let paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

/// SPECs section: placeholder list.
fn render_detail_specs(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let content = match state.selected_branch() {
        Some(branch) => format!(" SPECs for branch: {}\n\n No SPECs loaded", branch.name),
        None => " No branch selected".to_string(),
    };

    let block = Block::default().borders(Borders::ALL).title("SPECs");
    let paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

/// Git Status section: placeholder.
fn render_detail_git_status(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let content = match state.selected_branch() {
        Some(branch) => format!(" Git status for {}", branch.name),
        None => " No branch selected".to_string(),
    };

    let block = Block::default().borders(Borders::ALL).title("Git Status");
    let paragraph = Paragraph::new(content)
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);
}

/// Sessions section: placeholder.
fn render_detail_sessions(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Sessions");
    let paragraph = Paragraph::new(" No active sessions")
        .block(block)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

/// Action modal: centered overlay with selectable action list.
fn render_action_modal(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let dialog = super::centered_rect(30, 7, area);

    frame.render_widget(Clear, dialog);

    let items: Vec<ListItem> = ACTION_LABELS
        .iter()
        .enumerate()
        .map(|(idx, label)| {
            let style = super::list_item_style(idx == state.action_modal_selected);
            let prefix = if idx == state.action_modal_selected {
                "\u{25B6} "
            } else {
                "  "
            };
            ListItem::new(Line::from(Span::styled(format!("{prefix}{label}"), style)))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Actions")
        .border_style(Style::default().fg(Color::Yellow));
    let list = List::new(items).block(block);
    frame.render_widget(list, dialog);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_branches() -> Vec<BranchItem> {
        vec![
            BranchItem {
                name: "main".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Main,
            },
            BranchItem {
                name: "develop".to_string(),
                is_head: true,
                is_local: true,
                category: BranchCategory::Develop,
            },
            BranchItem {
                name: "feature/login".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Feature,
            },
            BranchItem {
                name: "origin/feature/api".to_string(),
                is_head: false,
                is_local: false,
                category: BranchCategory::Feature,
            },
            BranchItem {
                name: "hotfix/crash".to_string(),
                is_head: false,
                is_local: true,
                category: BranchCategory::Other,
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = BranchesState::default();
        assert!(state.branches.is_empty());
        assert_eq!(state.selected, 0);
        assert_eq!(state.sort_mode, SortMode::Default);
        assert_eq!(state.view_mode, ViewMode::All);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        assert_eq!(state.selected, 0);

        update(&mut state, BranchesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        // Move to last
        for _ in 0..4 {
            update(&mut state, BranchesMessage::MoveDown);
        }
        // Should wrap to 0
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_up_wraps() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        assert_eq!(state.selected, 0);

        update(&mut state, BranchesMessage::MoveUp);
        assert_eq!(state.selected, 4); // wraps to last
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
        assert_eq!(state.view_mode, ViewMode::All);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Local);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::Remote);

        update(&mut state, BranchesMessage::ToggleView);
        assert_eq!(state.view_mode, ViewMode::All);
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
        assert_eq!(state.selected, 4); // clamped
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
    fn sort_name_returns_alphabetical_order() {
        let mut state = BranchesState::default();
        state.branches = sample_branches(); // main, develop, feature/login, origin/feature/api, hotfix/crash
        state.sort_mode = SortMode::Name;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "develop",
                "feature/login",
                "hotfix/crash",
                "main",
                "origin/feature/api",
            ]
        );
    }

    #[test]
    fn sort_date_returns_alphabetical_fallback() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.sort_mode = SortMode::Date;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        // Date falls back to alphabetical since no date field exists
        assert_eq!(
            names,
            vec![
                "develop",
                "feature/login",
                "hotfix/crash",
                "main",
                "origin/feature/api",
            ]
        );
    }

    #[test]
    fn sort_default_preserves_insertion_order() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.sort_mode = SortMode::Default;

        let filtered = state.filtered_branches();
        let names: Vec<&str> = filtered.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "main",
                "develop",
                "feature/login",
                "origin/feature/api",
                "hotfix/crash",
            ]
        );
    }

    #[test]
    fn search_then_navigate_selects_filtered_item() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();

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
    fn action_modal_defaults_to_hidden() {
        let state = BranchesState::default();
        assert!(!state.action_modal_visible);
        assert_eq!(state.action_modal_selected, 0);
    }

    #[test]
    fn open_action_modal_sets_visible() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        update(&mut state, BranchesMessage::OpenActionModal);
        assert!(state.action_modal_visible);
        assert_eq!(state.action_modal_selected, 0);
    }

    #[test]
    fn open_action_modal_ignored_when_empty() {
        let mut state = BranchesState::default();
        update(&mut state, BranchesMessage::OpenActionModal);
        assert!(!state.action_modal_visible);
    }

    #[test]
    fn close_action_modal_hides() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        update(&mut state, BranchesMessage::CloseActionModal);
        assert!(!state.action_modal_visible);
    }

    #[test]
    fn action_modal_down_cycles() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        update(&mut state, BranchesMessage::ActionModalDown);
        assert_eq!(state.action_modal_selected, 1);
        update(&mut state, BranchesMessage::ActionModalDown);
        assert_eq!(state.action_modal_selected, 2);
        update(&mut state, BranchesMessage::ActionModalDown);
        assert_eq!(state.action_modal_selected, 0); // wraps
    }

    #[test]
    fn action_modal_up_wraps() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        update(&mut state, BranchesMessage::ActionModalUp);
        assert_eq!(state.action_modal_selected, 2); // wraps to last
    }

    #[test]
    fn action_modal_select_launch_agent() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        state.action_modal_selected = 0;
        update(&mut state, BranchesMessage::ActionModalSelect);
        assert!(!state.action_modal_visible);
        assert!(state.pending_launch_agent);
    }

    #[test]
    fn action_modal_select_open_shell() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        state.action_modal_selected = 1;
        update(&mut state, BranchesMessage::ActionModalSelect);
        assert!(!state.action_modal_visible);
        assert!(state.pending_open_shell);
    }

    #[test]
    fn action_modal_select_delete_worktree() {
        let mut state = BranchesState::default();
        state.action_modal_visible = true;
        state.action_modal_selected = 2;
        update(&mut state, BranchesMessage::ActionModalSelect);
        assert!(!state.action_modal_visible);
        assert!(state.pending_delete_worktree);
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
    fn delete_worktree_sets_flag() {
        let mut state = BranchesState::default();
        assert!(!state.pending_delete_worktree);
        update(&mut state, BranchesMessage::DeleteWorktree);
        assert!(state.pending_delete_worktree);
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
    fn render_detail_overview_shows_branch_info() {
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
        // Check bottom half contains Overview content
        let mut found_overview = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("Overview") {
                found_overview = true;
                break;
            }
        }
        assert!(found_overview, "Detail panel should contain 'Overview'");
    }

    #[test]
    fn render_action_modal_shows_action_list() {
        let mut state = BranchesState::default();
        state.branches = sample_branches();
        state.action_modal_visible = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area, BranchesFocus::List);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut found_actions = false;
        let mut found_launch = false;
        for y in 0..buf.area.height {
            let line: String = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            if line.contains("Actions") {
                found_actions = true;
            }
            if line.contains("Launch Agent") {
                found_launch = true;
            }
        }
        assert!(found_actions, "Should contain 'Actions' title");
        assert!(found_launch, "Should contain 'Launch Agent' action");
    }

    #[test]
    fn render_detail_sections_shows_correct_tab_labels() {
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
        let mut full_text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                full_text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            full_text.contains("SPECs"),
            "Tab bar should contain 'SPECs'"
        );
        assert!(
            full_text.contains("Sessions"),
            "Tab bar should contain 'Sessions'"
        );
        // Actions is no longer a tab section — it's an overlay modal
        assert!(
            !full_text.contains("Actions"),
            "Tab bar should NOT contain 'Actions'"
        );
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
}
