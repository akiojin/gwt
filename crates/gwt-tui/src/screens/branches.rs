//! Branches management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
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

/// State for the branches screen.
#[derive(Debug, Clone, Default)]
pub struct BranchesState {
    pub(crate) branches: Vec<BranchItem>,
    pub(crate) selected: usize,
    pub(crate) sort_mode: SortMode,
    pub(crate) view_mode: ViewMode,
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
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
            // Selection action — handled by caller via selected_branch()
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
    }
}

/// Render the branches screen.
pub fn render(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header: view/sort/search
            Constraint::Min(0),    // Branch list
        ])
        .split(area);

    // Header
    render_header(state, frame, chunks[0]);

    // Branch list
    render_branch_list(state, frame, chunks[1]);
}

/// Render the header bar with view mode, sort mode, and search.
fn render_header(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let search_display = if state.search_active {
        format!(" Search: {}_", state.search_query)
    } else if !state.search_query.is_empty() {
        format!(" Search: {}", state.search_query)
    } else {
        String::new()
    };

    let header_text = format!(
        " View: {} | Sort: {}{}",
        state.view_mode.label(),
        state.sort_mode.label(),
        search_display,
    );

    let block = Block::default().borders(Borders::ALL).title("Branches");
    let paragraph = Paragraph::new(header_text)
        .block(block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(paragraph, area);
}

/// Render the branch list grouped by category.
fn render_branch_list(state: &BranchesState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_branches();

    if filtered.is_empty() {
        super::render_empty_list(frame, area, !state.branches.is_empty(), "branches");
        return;
    }

    // Build items with category headers
    let mut items: Vec<ListItem> = Vec::new();
    let mut current_category: Option<BranchCategory> = None;

    for (idx, branch) in filtered.iter().enumerate() {
        if current_category != Some(branch.category) {
            current_category = Some(branch.category);
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

        let style = super::list_item_style(idx == state.selected);

        let line = Line::from(vec![
            Span::styled(
                head_indicator.to_string(),
                Style::default().fg(Color::Green),
            ),
            Span::styled(format!("[{}] ", locality), Style::default().fg(Color::Cyan)),
            Span::styled(branch.name.clone(), style),
        ]);
        items.push(ListItem::new(line));
    }

    let block = Block::default().borders(Borders::ALL);
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
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
                render(&state, f, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Branches"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = BranchesState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
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
}
