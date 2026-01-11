//! Branch List Screen

use gwt_core::git::{Branch, DivergenceStatus};
use gwt_core::worktree::Worktree;
use ratatui::{prelude::*, widgets::*};

/// Branch list state
#[derive(Debug, Default)]
pub struct BranchListState {
    pub branches: Vec<BranchItem>,
    pub selected: usize,
    pub offset: usize,
    pub filter: String,
    pub is_loading: bool,
}

/// Branch item with worktree info
#[derive(Debug, Clone)]
pub struct BranchItem {
    pub name: String,
    pub is_current: bool,
    pub has_worktree: bool,
    pub worktree_path: Option<String>,
    pub has_changes: bool,
    pub has_unpushed: bool,
    pub divergence: DivergenceStatus,
}

impl BranchItem {
    pub fn from_branch(branch: &Branch, worktrees: &[Worktree]) -> Self {
        let worktree = worktrees.iter().find(|wt| {
            wt.branch.as_ref().map(|b| b == &branch.name).unwrap_or(false)
        });

        Self {
            name: branch.name.clone(),
            is_current: branch.is_current,
            has_worktree: worktree.is_some(),
            worktree_path: worktree.map(|wt| wt.path.display().to_string()),
            has_changes: worktree.map(|wt| wt.has_changes).unwrap_or(false),
            has_unpushed: worktree.map(|wt| wt.has_unpushed).unwrap_or(false),
            divergence: DivergenceStatus::UpToDate, // TODO: Get from branch
        }
    }
}

impl BranchListState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_branches(mut self, branches: Vec<BranchItem>) -> Self {
        self.branches = branches;
        self
    }

    /// Get filtered branches
    pub fn filtered_branches(&self) -> Vec<&BranchItem> {
        if self.filter.is_empty() {
            self.branches.iter().collect()
        } else {
            let filter_lower = self.filter.to_lowercase();
            self.branches
                .iter()
                .filter(|b| b.name.to_lowercase().contains(&filter_lower))
                .collect()
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
        // Adjust offset if needed (visible window of 10 items by default)
        let visible_window = 10;
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
}

/// Render branch list
pub fn render_branch_list(
    state: &BranchListState,
    frame: &mut Frame,
    area: Rect,
) {
    let filtered = state.filtered_branches();

    if filtered.is_empty() {
        let text = if state.filter.is_empty() {
            "No branches found"
        } else {
            "No branches match filter"
        };
        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title(" Branches "));
        frame.render_widget(paragraph, area);
        return;
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .skip(state.offset)
        .take(visible_height)
        .map(|(i, branch)| render_branch_item(branch, i == state.selected))
        .collect();

    let title = if state.filter.is_empty() {
        format!(" Branches ({}) ", filtered.len())
    } else {
        format!(" Branches ({}/{}) [{}] ", filtered.len(), state.branches.len(), state.filter)
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_widget(list, area);

    // Render scrollbar
    if filtered.len() > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("^"))
            .end_symbol(Some("v"));
        let mut scrollbar_state = ScrollbarState::new(filtered.len())
            .position(state.selected);
        frame.render_stateful_widget(
            scrollbar,
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut scrollbar_state,
        );
    }
}

/// Render a single branch item
fn render_branch_item(branch: &BranchItem, is_selected: bool) -> ListItem<'static> {
    let mut spans = Vec::new();

    // Current branch indicator
    if branch.is_current {
        spans.push(Span::styled("* ", Style::default().fg(Color::Green)));
    } else {
        spans.push(Span::raw("  "));
    }

    // Branch name
    let name_style = if branch.has_worktree {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    spans.push(Span::styled(branch.name.clone(), name_style));

    // Status indicators
    if branch.has_worktree {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("[wt]", Style::default().fg(Color::Blue)));
    }

    if branch.has_changes {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("[dirty]", Style::default().fg(Color::Yellow)));
    }

    if branch.has_unpushed {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("[unpushed]", Style::default().fg(Color::Magenta)));
    }

    // Divergence status
    match &branch.divergence {
        DivergenceStatus::Ahead(n) => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("+{}", n),
                Style::default().fg(Color::Green),
            ));
        }
        DivergenceStatus::Behind(n) => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("-{}", n),
                Style::default().fg(Color::Red),
            ));
        }
        DivergenceStatus::Diverged { ahead, behind } => {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("+{} -{}", ahead, behind),
                Style::default().fg(Color::Yellow),
            ));
        }
        _ => {}
    }

    let style = if is_selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    ListItem::new(Line::from(spans)).style(style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branch_list_navigation() {
        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                is_current: true,
                has_worktree: true,
                worktree_path: Some("/path".to_string()),
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
            },
            BranchItem {
                name: "develop".to_string(),
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_next(); // Should not go beyond
        assert_eq!(state.selected, 1);

        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_branch_filter() {
        let branches = vec![
            BranchItem {
                name: "main".to_string(),
                is_current: true,
                has_worktree: true,
                worktree_path: None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
            },
            BranchItem {
                name: "feature/test".to_string(),
                is_current: false,
                has_worktree: false,
                worktree_path: None,
                has_changes: false,
                has_unpushed: false,
                divergence: DivergenceStatus::UpToDate,
            },
        ];

        let mut state = BranchListState::new().with_branches(branches);
        assert_eq!(state.filtered_branches().len(), 2);

        state.set_filter("feature".to_string());
        assert_eq!(state.filtered_branches().len(), 1);
        assert_eq!(state.filtered_branches()[0].name, "feature/test");
    }
}
