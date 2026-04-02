//! Issues management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// A single issue entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueItem {
    pub number: u32,
    pub title: String,
    pub state: String,
    pub labels: Vec<String>,
    pub body: String,
}

/// State for the issues screen.
#[derive(Debug, Clone, Default)]
pub struct IssuesState {
    pub issues: Vec<IssueItem>,
    pub selected: usize,
    pub detail_view: bool,
    pub search_query: String,
    pub search_active: bool,
}

impl IssuesState {
    /// Return issues filtered by the current search query.
    pub fn filtered_issues(&self) -> Vec<&IssueItem> {
        let query_lower = self.search_query.to_lowercase();
        self.issues
            .iter()
            .filter(|i| {
                query_lower.is_empty()
                    || i.title.to_lowercase().contains(&query_lower)
                    || i.labels.iter().any(|l| l.to_lowercase().contains(&query_lower))
                    || i.number.to_string().contains(&query_lower)
            })
            .collect()
    }

    /// Get the currently selected issue (from filtered list).
    pub fn selected_issue(&self) -> Option<&IssueItem> {
        let filtered = self.filtered_issues();
        filtered.get(self.selected).copied()
    }

    /// Clamp selected index to filtered length.
    fn clamp_selected(&mut self) {
        let len = self.filtered_issues().len();
        super::clamp_index(&mut self.selected, len);
    }
}

/// Messages specific to the issues screen.
#[derive(Debug, Clone)]
pub enum IssuesMessage {
    MoveUp,
    MoveDown,
    ToggleDetail,
    SearchStart,
    SearchInput(char),
    SearchClear,
    Refresh,
    SetIssues(Vec<IssueItem>),
}

/// Update issues state in response to a message.
pub fn update(state: &mut IssuesState, msg: IssuesMessage) {
    match msg {
        IssuesMessage::MoveUp => {
            let len = state.filtered_issues().len();
            super::move_up(&mut state.selected, len);
        }
        IssuesMessage::MoveDown => {
            let len = state.filtered_issues().len();
            super::move_down(&mut state.selected, len);
        }
        IssuesMessage::ToggleDetail => {
            if !state.filtered_issues().is_empty() {
                state.detail_view = !state.detail_view;
            }
        }
        IssuesMessage::SearchStart => {
            state.search_active = true;
        }
        IssuesMessage::SearchInput(ch) => {
            if state.search_active {
                state.search_query.push(ch);
                state.clamp_selected();
            }
        }
        IssuesMessage::SearchClear => {
            state.search_query.clear();
            state.search_active = false;
            state.clamp_selected();
        }
        IssuesMessage::Refresh => {
            // Signal to reload issues -- handled by caller
        }
        IssuesMessage::SetIssues(issues) => {
            state.issues = issues;
            state.clamp_selected();
        }
    }
}

/// Render the issues screen.
pub fn render(state: &IssuesState, frame: &mut Frame, area: Rect) {
    if state.detail_view {
        render_detail(state, frame, area);
    } else {
        render_list_view(state, frame, area);
    }
}

/// Render the list view with header and issue list.
fn render_list_view(state: &IssuesState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header: search bar
            Constraint::Min(0),   // Issue list
        ])
        .split(area);

    render_header(state, frame, chunks[0]);
    render_issue_list(state, frame, chunks[1]);
}

/// Render the header bar with search.
fn render_header(state: &IssuesState, frame: &mut Frame, area: Rect) {
    let search_display = if state.search_active {
        format!(" Search: {}_", state.search_query)
    } else if !state.search_query.is_empty() {
        format!(" Search: {}", state.search_query)
    } else {
        " Press '/' to search".to_string()
    };

    let count = state.filtered_issues().len();
    let total = state.issues.len();
    let header_text = format!(" Issues ({}/{})  |{}", count, total, search_display);

    let block = Block::default().borders(Borders::ALL).title("Issues");
    let paragraph = Paragraph::new(header_text)
        .block(block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(paragraph, area);
}

/// Render the issue list.
fn render_issue_list(state: &IssuesState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_issues();

    if filtered.is_empty() {
        super::render_empty_list(frame, area, !state.issues.is_empty(), "issues");
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, issue)| {
            let state_color = match issue.state.as_str() {
                "open" => Color::Green,
                "closed" => Color::Red,
                _ => Color::DarkGray,
            };

            let style = super::list_item_style(idx == state.selected);

            let labels_str = if issue.labels.is_empty() {
                String::new()
            } else {
                format!(" [{}]", issue.labels.join(", "))
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("#{:<5} ", issue.number),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(issue.title.clone(), style),
                Span::styled(
                    format!(" ({})", issue.state),
                    Style::default().fg(state_color),
                ),
                Span::styled(labels_str, Style::default().fg(Color::Magenta)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().borders(Borders::ALL);
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the detail view for the selected issue.
fn render_detail(state: &IssuesState, frame: &mut Frame, area: Rect) {
    let issue = match state.selected_issue() {
        Some(i) => i,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Issue Detail");
            let paragraph = Paragraph::new("No issue selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Issue header
            Constraint::Min(0),   // Body
        ])
        .split(area);

    // Header section
    let labels_str = if issue.labels.is_empty() {
        "None".to_string()
    } else {
        issue.labels.join(", ")
    };

    let header_text = format!(
        " #{} - {}\n State: {} | Labels: {}\n Press Enter to go back",
        issue.number, issue.title, issue.state, labels_str,
    );
    let header_block = Block::default()
        .borders(Borders::ALL)
        .title("Issue Detail");
    let header = Paragraph::new(header_text)
        .block(header_block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(header, chunks[0]);

    // Body section
    let body_block = Block::default().borders(Borders::ALL).title("Description");
    let body_text = if issue.body.is_empty() {
        "No description provided.".to_string()
    } else {
        issue.body.clone()
    };
    let body = Paragraph::new(body_text)
        .block(body_block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));
    frame.render_widget(body, chunks[1]);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_issues() -> Vec<IssueItem> {
        vec![
            IssueItem {
                number: 1,
                title: "Fix login bug".to_string(),
                state: "open".to_string(),
                labels: vec!["bug".to_string(), "priority".to_string()],
                body: "Login fails on Safari.".to_string(),
            },
            IssueItem {
                number: 2,
                title: "Add dark mode".to_string(),
                state: "open".to_string(),
                labels: vec!["enhancement".to_string()],
                body: "Users want dark mode support.".to_string(),
            },
            IssueItem {
                number: 3,
                title: "Update README".to_string(),
                state: "closed".to_string(),
                labels: vec![],
                body: String::new(),
            },
            IssueItem {
                number: 10,
                title: "Refactor settings".to_string(),
                state: "open".to_string(),
                labels: vec!["refactor".to_string()],
                body: "Settings module needs cleanup.".to_string(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = IssuesState::default();
        assert!(state.issues.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.detail_view);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();

        update(&mut state, IssuesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        for _ in 0..3 {
            update(&mut state, IssuesMessage::MoveDown);
        }
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();

        update(&mut state, IssuesMessage::MoveUp);
        assert_eq!(state.selected, 3); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = IssuesState::default();
        update(&mut state, IssuesMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, IssuesMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_detail_flips() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        assert!(!state.detail_view);

        update(&mut state, IssuesMessage::ToggleDetail);
        assert!(state.detail_view);

        update(&mut state, IssuesMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn toggle_detail_noop_on_empty() {
        let mut state = IssuesState::default();
        update(&mut state, IssuesMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn search_start_activates() {
        let mut state = IssuesState::default();
        update(&mut state, IssuesMessage::SearchStart);
        assert!(state.search_active);
    }

    #[test]
    fn search_input_appends() {
        let mut state = IssuesState::default();
        update(&mut state, IssuesMessage::SearchStart);
        update(&mut state, IssuesMessage::SearchInput('b'));
        update(&mut state, IssuesMessage::SearchInput('u'));
        assert_eq!(state.search_query, "bu");
    }

    #[test]
    fn search_input_ignored_when_inactive() {
        let mut state = IssuesState::default();
        update(&mut state, IssuesMessage::SearchInput('x'));
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn search_clear_resets() {
        let mut state = IssuesState::default();
        state.search_active = true;
        state.search_query = "test".to_string();

        update(&mut state, IssuesMessage::SearchClear);
        assert!(!state.search_active);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn set_issues_populates() {
        let mut state = IssuesState::default();
        state.selected = 99;
        update(&mut state, IssuesMessage::SetIssues(sample_issues()));
        assert_eq!(state.issues.len(), 4);
        assert_eq!(state.selected, 3); // clamped
    }

    #[test]
    fn filtered_issues_respects_search() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        state.search_query = "bug".to_string();

        let filtered = state.filtered_issues();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].number, 1);
    }

    #[test]
    fn filtered_issues_search_by_number() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        state.search_query = "10".to_string();

        let filtered = state.filtered_issues();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].number, 10);
    }

    #[test]
    fn filtered_issues_search_by_label() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        state.search_query = "enhancement".to_string();

        let filtered = state.filtered_issues();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].number, 2);
    }

    #[test]
    fn selected_issue_returns_correct() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        state.selected = 2;
        let issue = state.selected_issue().unwrap();
        assert_eq!(issue.number, 3);
    }

    #[test]
    fn render_list_does_not_panic() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
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
        assert!(text.contains("Issues"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = IssuesState::default();
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
    fn render_detail_does_not_panic() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        state.detail_view = true;
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
    fn render_detail_empty_body_does_not_panic() {
        let mut state = IssuesState::default();
        state.issues = vec![IssueItem {
            number: 99,
            title: "No body".to_string(),
            state: "open".to_string(),
            labels: vec![],
            body: String::new(),
        }];
        state.detail_view = true;
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
    fn search_clamps_selected_when_filtering() {
        let mut state = IssuesState::default();
        state.issues = sample_issues();
        state.selected = 3; // last item

        update(&mut state, IssuesMessage::SearchStart);
        // Search narrows to 1 result
        update(&mut state, IssuesMessage::SearchInput('r'));
        update(&mut state, IssuesMessage::SearchInput('e'));
        update(&mut state, IssuesMessage::SearchInput('a'));
        update(&mut state, IssuesMessage::SearchInput('d'));
        // "read" matches "Update README"
        let filtered = state.filtered_issues();
        assert!(state.selected < filtered.len().max(1));
    }
}
