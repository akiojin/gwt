use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget},
};

#[derive(Debug, Clone, PartialEq)]
pub enum IssueType {
    Spec(String), // e.g. "SPEC-1776"
    Issue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Debug, Clone)]
pub struct IssueEntry {
    pub number: u64,
    pub title: String,
    pub issue_type: IssueType,
    pub state: IssueState,
    pub labels: Vec<String>,
}

#[derive(Debug, Default)]
pub struct IssuePanelState {
    pub issues: Vec<IssueEntry>,
    pub filtered: Vec<usize>, // indices into `issues`
    pub search_query: String,
    pub selected_index: usize,
    pub search_focused: bool,
}

impl IssuePanelState {
    /// Rebuild `filtered` from `search_query`. An empty query shows all issues.
    pub fn update_filter(&mut self) {
        let query = self.search_query.to_lowercase();
        self.filtered = self
            .issues
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if query.is_empty() {
                    return true;
                }
                if entry.title.to_lowercase().contains(&query) {
                    return true;
                }
                if let IssueType::Spec(ref id) = entry.issue_type {
                    if id.to_lowercase().contains(&query) {
                        return true;
                    }
                }
                let num_str = entry.number.to_string();
                if num_str.contains(&query) {
                    return true;
                }
                false
            })
            .map(|(i, _)| i)
            .collect();

        // Keep selected_index in range after filtering.
        if !self.filtered.is_empty() && self.selected_index >= self.filtered.len() {
            self.selected_index = self.filtered.len() - 1;
        }
    }

    pub fn select_next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.filtered.len();
    }

    pub fn select_prev(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.filtered.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    pub fn selected_issue(&self) -> Option<&IssueEntry> {
        self.filtered
            .get(self.selected_index)
            .and_then(|&idx| self.issues.get(idx))
    }

    pub fn handle_search_input(&mut self, c: char) {
        self.search_query.push(c);
        self.update_filter();
    }

    pub fn handle_search_backspace(&mut self) {
        self.search_query.pop();
        self.update_filter();
    }
}

pub fn render(buf: &mut Buffer, area: Rect, state: &IssuePanelState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Issues & SPECs ");
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height < 2 || inner.width < 4 {
        return;
    }

    let search_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let search_line = Line::from(vec![
        Span::styled("Search: ", Style::default().fg(Color::Yellow)),
        Span::raw(&state.search_query),
    ]);
    Paragraph::new(search_line).render(search_area, buf);

    if inner.height < 3 {
        return;
    }
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height - 1,
    };

    if state.filtered.is_empty() {
        let empty = Paragraph::new("  No issues found.");
        empty.render(list_area, buf);
        return;
    }

    let items: Vec<ListItem> = state
        .filtered
        .iter()
        .enumerate()
        .map(|(list_idx, &issue_idx)| {
            let entry = &state.issues[issue_idx];
            let is_selected = list_idx == state.selected_index;

            let type_span = match &entry.issue_type {
                IssueType::Spec(id) => {
                    Span::styled(format!("{:<12}", id), Style::default().fg(Color::Cyan))
                }
                IssueType::Issue => Span::styled(
                    format!("{:<12}", "Issue"),
                    Style::default().fg(Color::White),
                ),
            };

            let state_color = match entry.state {
                IssueState::Open => Color::Green,
                IssueState::Closed => Color::DarkGray,
            };
            let state_label = match entry.state {
                IssueState::Open => "open",
                IssueState::Closed => "closed",
            };

            let line = Line::from(vec![
                Span::raw(format!("#{:<5} ", entry.number)),
                type_span,
                Span::raw(format!("{:<24} ", truncate(&entry.title, 24))),
                Span::styled(state_label, Style::default().fg(state_color)),
            ]);

            let mut style = Style::default();
            if is_selected {
                style = style
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD);
            }
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items);
    list.render(list_area, buf);
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
/// Uses char boundaries to avoid panics on multi-byte UTF-8.
fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_issues() -> Vec<IssueEntry> {
        vec![
            IssueEntry {
                number: 42,
                title: "TUI Migration".to_string(),
                issue_type: IssueType::Spec("SPEC-1776".to_string()),
                state: IssueState::Open,
                labels: vec!["enhancement".to_string()],
            },
            IssueEntry {
                number: 38,
                title: "Voice Input".to_string(),
                issue_type: IssueType::Spec("SPEC-1654".to_string()),
                state: IssueState::Open,
                labels: vec![],
            },
            IssueEntry {
                number: 35,
                title: "Fix timeout".to_string(),
                issue_type: IssueType::Issue,
                state: IssueState::Closed,
                labels: vec!["bug".to_string()],
            },
        ]
    }

    fn make_state() -> IssuePanelState {
        let issues = sample_issues();
        let filtered: Vec<usize> = (0..issues.len()).collect();
        IssuePanelState {
            issues,
            filtered,
            search_query: String::new(),
            selected_index: 0,
            search_focused: false,
        }
    }

    #[test]
    fn test_issue_panel_default_state() {
        let state = IssuePanelState::default();
        assert!(state.issues.is_empty());
        assert!(state.filtered.is_empty());
        assert_eq!(state.search_query, "");
        assert_eq!(state.selected_index, 0);
        assert!(!state.search_focused);
    }

    #[test]
    fn test_update_filter_matches_title() {
        let mut state = make_state();
        state.search_query = "timeout".to_string();
        state.update_filter();
        assert_eq!(state.filtered.len(), 1);
        assert_eq!(state.issues[state.filtered[0]].number, 35);
    }

    #[test]
    fn test_update_filter_matches_spec_id() {
        let mut state = make_state();
        state.search_query = "1776".to_string();
        state.update_filter();
        assert_eq!(state.filtered.len(), 1);
        assert_eq!(state.issues[state.filtered[0]].number, 42);
    }

    #[test]
    fn test_update_filter_empty_query_shows_all() {
        let mut state = make_state();
        state.search_query = String::new();
        state.update_filter();
        assert_eq!(state.filtered.len(), 3);
    }

    #[test]
    fn test_select_next_wraps() {
        let mut state = make_state();
        assert_eq!(state.selected_index, 0);
        state.select_next();
        assert_eq!(state.selected_index, 1);
        state.select_next();
        assert_eq!(state.selected_index, 2);
        state.select_next(); // wraps
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn test_select_prev_wraps() {
        let mut state = make_state();
        assert_eq!(state.selected_index, 0);
        state.select_prev(); // wraps to end
        assert_eq!(state.selected_index, 2);
        state.select_prev();
        assert_eq!(state.selected_index, 1);
    }

    #[test]
    fn test_selected_issue_empty() {
        let state = IssuePanelState::default();
        assert!(state.selected_issue().is_none());
    }

    #[test]
    fn test_handle_search_input() {
        let mut state = make_state();
        state.handle_search_input('t');
        assert_eq!(state.search_query, "t");
        state.handle_search_input('u');
        assert_eq!(state.search_query, "tu");
        // "TUI Migration" matches "tu"
        assert!(state.filtered.len() >= 1);
    }

    #[test]
    fn test_handle_search_backspace() {
        let mut state = make_state();
        state.search_query = "tui".to_string();
        state.update_filter();
        state.handle_search_backspace();
        assert_eq!(state.search_query, "tu");
        // Still filters after backspace
        state.handle_search_backspace();
        state.handle_search_backspace();
        assert_eq!(state.search_query, "");
        assert_eq!(state.filtered.len(), 3); // all shown
    }

    #[test]
    fn test_render_empty_list() {
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = IssuePanelState::default();

        terminal
            .draw(|frame| {
                let area = frame.area();
                render(frame.buffer_mut(), area, &state);
            })
            .unwrap();

        let buf_str = terminal.backend().buffer().content().iter().fold(
            String::new(),
            |mut acc, cell| {
                acc.push_str(cell.symbol());
                acc
            },
        );
        assert!(buf_str.contains("Issues & SPECs"));
        assert!(buf_str.contains("No issues found"));
    }

    #[test]
    fn test_render_with_issues() {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state();

        terminal
            .draw(|frame| {
                let area = frame.area();
                render(frame.buffer_mut(), area, &state);
            })
            .unwrap();

        let buf_str = terminal.backend().buffer().content().iter().fold(
            String::new(),
            |mut acc, cell| {
                acc.push_str(cell.symbol());
                acc
            },
        );
        assert!(buf_str.contains("Issues & SPECs"));
        assert!(buf_str.contains("Search:"));
        assert!(buf_str.contains("#42"));
        assert!(buf_str.contains("SPEC-1776"));
        assert!(buf_str.contains("TUI Migration"));
        assert!(buf_str.contains("open"));
    }
}
