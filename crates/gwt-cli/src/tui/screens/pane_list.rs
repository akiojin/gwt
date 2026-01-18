//! Pane list component for tmux multi-mode
//!
//! Displays a list of running agent panes with branch name, agent name, uptime, and state.

use gwt_core::tmux::AgentPane;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

/// State for the pane list component
/// Note: PaneList panel is abolished, but this struct is still used for agent tracking
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct PaneListState {
    /// List of running agent panes
    pub panes: Vec<AgentPane>,
    /// Currently selected pane index
    pub selected: usize,
    /// List state for ratatui
    list_state: ListState,
    /// Whether this component has focus (deprecated)
    pub has_focus: bool,
    /// Spinner animation frame counter (FR-031a-b)
    pub spinner_frame: usize,
}

impl PaneListState {
    /// Create a new pane list state
    pub fn new() -> Self {
        Self {
            panes: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            has_focus: false,
            spinner_frame: 0,
        }
    }

    /// Update the list of panes
    pub fn update_panes(&mut self, panes: Vec<AgentPane>) {
        self.panes = panes;
        // Adjust selection if needed
        if self.selected >= self.panes.len() && !self.panes.is_empty() {
            self.selected = self.panes.len() - 1;
        }
        self.list_state.select(if self.panes.is_empty() {
            None
        } else {
            Some(self.selected)
        });
    }

    /// Select the next pane (deprecated - PaneList panel abolished)
    #[allow(dead_code)]
    pub fn select_next(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.panes.len();
        self.list_state.select(Some(self.selected));
    }

    /// Select the previous pane (deprecated - PaneList panel abolished)
    #[allow(dead_code)]
    pub fn select_prev(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.panes.len() - 1
        } else {
            self.selected - 1
        };
        self.list_state.select(Some(self.selected));
    }

    /// Get the currently selected pane
    #[allow(dead_code)]
    pub fn selected_pane(&self) -> Option<&AgentPane> {
        self.panes.get(self.selected)
    }

    /// Check if the list is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    /// Get the number of panes
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.panes.len()
    }
}

/// Render the pane list (deprecated - PaneList panel abolished)
#[allow(dead_code)]
pub fn render_pane_list(state: &mut PaneListState, frame: &mut Frame, area: Rect) {
    let border_style = if state.has_focus {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" Agents ({}) ", state.panes.len()));

    if state.panes.is_empty() {
        let empty_text = Line::from(vec![Span::styled(
            "No agents running",
            Style::default().fg(Color::DarkGray),
        )]);
        let list = List::new(vec![ListItem::new(empty_text)]).block(block);
        frame.render_widget(list, area);
        return;
    }

    let spinner_frame = state.spinner_frame;
    let items: Vec<ListItem> = state
        .panes
        .iter()
        .enumerate()
        .map(|(i, pane)| {
            let is_selected = i == state.selected && state.has_focus;
            create_pane_list_item(pane, is_selected, spinner_frame)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut state.list_state);
}

/// Create a list item for a pane (deprecated - PaneList panel abolished)
#[allow(dead_code)]
fn create_pane_list_item(pane: &AgentPane, _is_selected: bool, _spinner_frame: usize) -> ListItem<'static> {
    let uptime = pane.uptime_string();

    // Show [BG] indicator for background (hidden) panes
    let status_indicator = if pane.is_background {
        Span::styled("[BG] ", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw("")
    };

    let spans = vec![
        status_indicator,
        Span::styled(
            format!(
                "{:<20}",
                truncate_string(&pane.branch_name, if pane.is_background { 15 } else { 20 })
            ),
            Style::default().fg(if pane.is_background {
                Color::DarkGray
            } else {
                Color::Green
            }),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<10}", pane.agent_name),
            Style::default().fg(if pane.is_background {
                Color::DarkGray
            } else {
                Color::Cyan
            }),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:>8}", uptime),
            Style::default().fg(if pane.is_background {
                Color::DarkGray
            } else {
                Color::Yellow
            }),
        ),
    ];

    ListItem::new(Line::from(spans))
}

/// Truncate a string to a maximum length (deprecated - PaneList panel abolished)
#[allow(dead_code)]
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn create_test_pane(branch: &str, agent: &str, secs_ago: u64) -> AgentPane {
        AgentPane::new(
            "1".to_string(),
            branch.to_string(),
            agent.to_string(),
            SystemTime::now() - Duration::from_secs(secs_ago),
            12345,
        )
    }

    #[test]
    fn test_pane_list_state_new() {
        let state = PaneListState::new();
        assert!(state.panes.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.has_focus);
    }

    #[test]
    fn test_pane_list_state_update_panes() {
        let mut state = PaneListState::new();
        let panes = vec![
            create_test_pane("feature/a", "claude", 60),
            create_test_pane("feature/b", "codex", 120),
        ];
        state.update_panes(panes);
        assert_eq!(state.panes.len(), 2);
    }

    #[test]
    fn test_pane_list_state_select_next() {
        let mut state = PaneListState::new();
        state.update_panes(vec![
            create_test_pane("a", "claude", 0),
            create_test_pane("b", "codex", 0),
            create_test_pane("c", "gemini", 0),
        ]);

        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 2);
        state.select_next();
        assert_eq!(state.selected, 0); // wrap around
    }

    #[test]
    fn test_pane_list_state_select_prev() {
        let mut state = PaneListState::new();
        state.update_panes(vec![
            create_test_pane("a", "claude", 0),
            create_test_pane("b", "codex", 0),
        ]);

        assert_eq!(state.selected, 0);
        state.select_prev();
        assert_eq!(state.selected, 1); // wrap around
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_pane_list_state_selected_pane() {
        let mut state = PaneListState::new();
        state.update_panes(vec![
            create_test_pane("feature/a", "claude", 0),
            create_test_pane("feature/b", "codex", 0),
        ]);

        let selected = state.selected_pane().unwrap();
        assert_eq!(selected.branch_name, "feature/a");

        state.select_next();
        let selected = state.selected_pane().unwrap();
        assert_eq!(selected.branch_name, "feature/b");
    }

    #[test]
    fn test_pane_list_state_empty() {
        let state = PaneListState::new();
        assert!(state.is_empty());
        assert_eq!(state.len(), 0);
        assert!(state.selected_pane().is_none());
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("short", 10), "short");
        assert_eq!(truncate_string("very long string", 10), "very lo...");
        assert_eq!(truncate_string("exact10", 10), "exact10");
    }

    #[test]
    fn test_pane_list_state_selection_adjustment() {
        let mut state = PaneListState::new();
        state.update_panes(vec![
            create_test_pane("a", "claude", 0),
            create_test_pane("b", "codex", 0),
            create_test_pane("c", "gemini", 0),
        ]);
        state.selected = 2;
        state.list_state.select(Some(2));

        // Remove all but one pane
        state.update_panes(vec![create_test_pane("a", "claude", 0)]);

        // Selection should be adjusted
        assert_eq!(state.selected, 0);
    }
}
