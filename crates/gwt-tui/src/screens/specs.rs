//! Specs management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Detail sections available for a spec.
const DETAIL_SECTIONS: [&str; 5] = [
    "spec.md",
    "plan.md",
    "tasks.md",
    "research.md",
    "data-model.md",
];

/// A single spec entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecItem {
    pub id: String,
    pub title: String,
    pub phase: String,
    pub status: String,
}

/// State for the specs screen.
#[derive(Debug, Clone, Default)]
pub struct SpecsState {
    pub specs: Vec<SpecItem>,
    pub selected: usize,
    pub detail_view: bool,
    pub detail_section: usize,
    pub search_query: String,
    pub search_active: bool,
}

impl SpecsState {
    /// Return specs filtered by the current search query.
    pub fn filtered_specs(&self) -> Vec<&SpecItem> {
        let query_lower = self.search_query.to_lowercase();
        self.specs
            .iter()
            .filter(|s| {
                query_lower.is_empty()
                    || s.id.to_lowercase().contains(&query_lower)
                    || s.title.to_lowercase().contains(&query_lower)
                    || s.phase.to_lowercase().contains(&query_lower)
                    || s.status.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// Get the currently selected spec (from filtered list).
    pub fn selected_spec(&self) -> Option<&SpecItem> {
        let filtered = self.filtered_specs();
        filtered.get(self.selected).copied()
    }

    /// Clamp selected index to filtered length.
    fn clamp_selected(&mut self) {
        let len = self.filtered_specs().len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }
}

/// Messages specific to the specs screen.
#[derive(Debug, Clone)]
pub enum SpecsMessage {
    MoveUp,
    MoveDown,
    ToggleDetail,
    NextSection,
    PrevSection,
    SearchStart,
    SearchInput(char),
    SearchClear,
    Refresh,
    SetSpecs(Vec<SpecItem>),
}

/// Update specs state in response to a message.
pub fn update(state: &mut SpecsState, msg: SpecsMessage) {
    match msg {
        SpecsMessage::MoveUp => {
            let len = state.filtered_specs().len();
            if len > 0 {
                state.selected = if state.selected == 0 {
                    len - 1
                } else {
                    state.selected - 1
                };
            }
        }
        SpecsMessage::MoveDown => {
            let len = state.filtered_specs().len();
            if len > 0 {
                state.selected = (state.selected + 1) % len;
            }
        }
        SpecsMessage::ToggleDetail => {
            if !state.filtered_specs().is_empty() {
                state.detail_view = !state.detail_view;
                if state.detail_view {
                    state.detail_section = 0;
                }
            }
        }
        SpecsMessage::NextSection => {
            if state.detail_view {
                state.detail_section = (state.detail_section + 1) % DETAIL_SECTIONS.len();
            }
        }
        SpecsMessage::PrevSection => {
            if state.detail_view {
                state.detail_section = if state.detail_section == 0 {
                    DETAIL_SECTIONS.len() - 1
                } else {
                    state.detail_section - 1
                };
            }
        }
        SpecsMessage::SearchStart => {
            state.search_active = true;
        }
        SpecsMessage::SearchInput(ch) => {
            if state.search_active {
                state.search_query.push(ch);
                state.clamp_selected();
            }
        }
        SpecsMessage::SearchClear => {
            state.search_query.clear();
            state.search_active = false;
            state.clamp_selected();
        }
        SpecsMessage::Refresh => {
            // Signal to reload specs -- handled by caller
        }
        SpecsMessage::SetSpecs(specs) => {
            state.specs = specs;
            state.clamp_selected();
        }
    }
}

/// Render the specs screen.
pub fn render(state: &SpecsState, frame: &mut Frame, area: Rect) {
    if state.detail_view {
        render_detail(state, frame, area);
    } else {
        render_list_view(state, frame, area);
    }
}

/// Render the list view with header and spec list.
fn render_list_view(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header: search bar
            Constraint::Min(0),   // Spec list
        ])
        .split(area);

    render_header(state, frame, chunks[0]);
    render_spec_list(state, frame, chunks[1]);
}

/// Render the header bar with search.
fn render_header(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let search_display = if state.search_active {
        format!(" Search: {}_", state.search_query)
    } else if !state.search_query.is_empty() {
        format!(" Search: {}", state.search_query)
    } else {
        " Press '/' to search".to_string()
    };

    let count = state.filtered_specs().len();
    let total = state.specs.len();
    let header_text = format!(" Specs ({}/{})  |{}", count, total, search_display);

    let block = Block::default().borders(Borders::ALL).title("Specs");
    let paragraph = Paragraph::new(header_text)
        .block(block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(paragraph, area);
}

/// Render the spec list.
fn render_spec_list(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let filtered = state.filtered_specs();

    if filtered.is_empty() {
        let block = Block::default().borders(Borders::ALL);
        let msg = if state.specs.is_empty() {
            "No specs loaded"
        } else {
            "No matching specs"
        };
        let paragraph = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, spec)| {
            let status_color = match spec.status.as_str() {
                "done" | "completed" => Color::Green,
                "in-progress" | "active" => Color::Yellow,
                "draft" | "planned" => Color::Cyan,
                "blocked" => Color::Red,
                _ => Color::DarkGray,
            };

            let style = if idx == state.selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{:<10} ", spec.id),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(spec.title.clone(), style),
                Span::styled(
                    format!(" [{}]", spec.phase),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(
                    format!(" ({})", spec.status),
                    Style::default().fg(status_color),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().borders(Borders::ALL);
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the detail view for the selected spec.
fn render_detail(state: &SpecsState, frame: &mut Frame, area: Rect) {
    let spec = match state.selected_spec() {
        Some(s) => s,
        None => {
            let block = Block::default().borders(Borders::ALL).title("Spec Detail");
            let paragraph = Paragraph::new("No spec selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Spec header
            Constraint::Length(3), // Section tabs
            Constraint::Min(0),   // Section content
        ])
        .split(area);

    // Header section
    let header_text = format!(
        " {} - {}\n Phase: {} | Status: {}\n Press Enter to go back | Tab/Shift+Tab: sections",
        spec.id, spec.title, spec.phase, spec.status,
    );
    let header_block = Block::default()
        .borders(Borders::ALL)
        .title("Spec Detail");
    let header = Paragraph::new(header_text)
        .block(header_block)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(header, chunks[0]);

    // Section tabs
    let section_titles: Vec<Line> = DETAIL_SECTIONS
        .iter()
        .map(|s| Line::from(*s))
        .collect();
    let section_tabs = ratatui::widgets::Tabs::new(section_titles)
        .block(Block::default().borders(Borders::ALL).title("Sections"))
        .select(state.detail_section)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(section_tabs, chunks[1]);

    // Section content placeholder
    let section_name = DETAIL_SECTIONS[state.detail_section];
    let content_text = format!(
        "Content of {} for {}\n\n(File content would be loaded here)",
        section_name, spec.id,
    );
    let content_block = Block::default()
        .borders(Borders::ALL)
        .title(section_name);
    let content = Paragraph::new(content_text)
        .block(content_block)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));
    frame.render_widget(content, chunks[2]);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_specs() -> Vec<SpecItem> {
        vec![
            SpecItem {
                id: "SPEC-1".to_string(),
                title: "Add worktree support".to_string(),
                phase: "implementation".to_string(),
                status: "in-progress".to_string(),
            },
            SpecItem {
                id: "SPEC-2".to_string(),
                title: "Agent orchestration".to_string(),
                phase: "planning".to_string(),
                status: "draft".to_string(),
            },
            SpecItem {
                id: "SPEC-3".to_string(),
                title: "Voice commands".to_string(),
                phase: "completed".to_string(),
                status: "done".to_string(),
            },
            SpecItem {
                id: "SPEC-10".to_string(),
                title: "Settings UI".to_string(),
                phase: "design".to_string(),
                status: "blocked".to_string(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = SpecsState::default();
        assert!(state.specs.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.detail_view);
        assert_eq!(state.detail_section, 0);
        assert!(state.search_query.is_empty());
        assert!(!state.search_active);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();

        update(&mut state, SpecsMessage::MoveDown);
        assert_eq!(state.selected, 1);

        for _ in 0..3 {
            update(&mut state, SpecsMessage::MoveDown);
        }
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();

        update(&mut state, SpecsMessage::MoveUp);
        assert_eq!(state.selected, 3); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, SpecsMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_detail_flips() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        assert!(!state.detail_view);

        update(&mut state, SpecsMessage::ToggleDetail);
        assert!(state.detail_view);
        assert_eq!(state.detail_section, 0); // reset on open

        update(&mut state, SpecsMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn toggle_detail_noop_on_empty() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::ToggleDetail);
        assert!(!state.detail_view);
    }

    #[test]
    fn next_section_cycles() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        assert_eq!(state.detail_section, 0);

        update(&mut state, SpecsMessage::NextSection);
        assert_eq!(state.detail_section, 1);

        // Cycle through all sections
        for _ in 0..4 {
            update(&mut state, SpecsMessage::NextSection);
        }
        assert_eq!(state.detail_section, 0); // wraps
    }

    #[test]
    fn prev_section_cycles() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        assert_eq!(state.detail_section, 0);

        update(&mut state, SpecsMessage::PrevSection);
        assert_eq!(state.detail_section, 4); // wraps to last
    }

    #[test]
    fn section_navigation_noop_outside_detail() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        assert!(!state.detail_view);

        update(&mut state, SpecsMessage::NextSection);
        assert_eq!(state.detail_section, 0);

        update(&mut state, SpecsMessage::PrevSection);
        assert_eq!(state.detail_section, 0);
    }

    #[test]
    fn search_start_activates() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::SearchStart);
        assert!(state.search_active);
    }

    #[test]
    fn search_input_appends() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::SearchStart);
        update(&mut state, SpecsMessage::SearchInput('w'));
        update(&mut state, SpecsMessage::SearchInput('o'));
        assert_eq!(state.search_query, "wo");
    }

    #[test]
    fn search_input_ignored_when_inactive() {
        let mut state = SpecsState::default();
        update(&mut state, SpecsMessage::SearchInput('x'));
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn search_clear_resets() {
        let mut state = SpecsState::default();
        state.search_active = true;
        state.search_query = "test".to_string();

        update(&mut state, SpecsMessage::SearchClear);
        assert!(!state.search_active);
        assert!(state.search_query.is_empty());
    }

    #[test]
    fn set_specs_populates() {
        let mut state = SpecsState::default();
        state.selected = 99;
        update(&mut state, SpecsMessage::SetSpecs(sample_specs()));
        assert_eq!(state.specs.len(), 4);
        assert_eq!(state.selected, 3); // clamped
    }

    #[test]
    fn filtered_specs_respects_search_by_title() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "worktree".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-1");
    }

    #[test]
    fn filtered_specs_search_by_id() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "SPEC-10".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-10");
    }

    #[test]
    fn filtered_specs_search_by_phase() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "planning".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-2");
    }

    #[test]
    fn filtered_specs_search_by_status() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.search_query = "blocked".to_string();

        let filtered = state.filtered_specs();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "SPEC-10");
    }

    #[test]
    fn selected_spec_returns_correct() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 2;
        let spec = state.selected_spec().unwrap();
        assert_eq!(spec.id, "SPEC-3");
    }

    #[test]
    fn render_list_does_not_panic() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
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
        assert!(text.contains("Specs"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = SpecsState::default();
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
        let mut state = SpecsState::default();
        state.specs = sample_specs();
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
    fn render_detail_with_section_does_not_panic() {
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.detail_view = true;
        state.detail_section = 3; // research.md
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
        let mut state = SpecsState::default();
        state.specs = sample_specs();
        state.selected = 3; // last item

        update(&mut state, SpecsMessage::SearchStart);
        update(&mut state, SpecsMessage::SearchInput('v'));
        update(&mut state, SpecsMessage::SearchInput('o'));
        update(&mut state, SpecsMessage::SearchInput('i'));
        update(&mut state, SpecsMessage::SearchInput('c'));
        update(&mut state, SpecsMessage::SearchInput('e'));
        // "voice" matches "Voice commands"
        let filtered = state.filtered_specs();
        assert!(state.selected < filtered.len().max(1));
    }

    #[test]
    fn detail_sections_constant_has_five_entries() {
        assert_eq!(DETAIL_SECTIONS.len(), 5);
        assert_eq!(DETAIL_SECTIONS[0], "spec.md");
        assert_eq!(DETAIL_SECTIONS[4], "data-model.md");
    }
}
