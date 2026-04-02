//! Versions screen — display git tags / releases.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

/// A single version tag entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionTag {
    pub name: String,
    pub date: String,
    pub message: String,
}

/// State for the versions screen.
#[derive(Debug, Clone, Default)]
pub struct VersionsState {
    pub(crate) tags: Vec<VersionTag>,
    pub(crate) selected: usize,
}

impl VersionsState {
    /// Get the currently selected tag, if any.
    pub fn selected_tag(&self) -> Option<&VersionTag> {
        self.tags.get(self.selected)
    }

    /// Clamp selected index to list length.
    fn clamp_selected(&mut self) {
        super::clamp_index(&mut self.selected, self.tags.len());
    }
}

/// Messages specific to the versions screen.
#[derive(Debug, Clone)]
pub enum VersionsMessage {
    MoveUp,
    MoveDown,
    Refresh,
    SetTags(Vec<VersionTag>),
}

/// Update versions state in response to a message.
pub fn update(state: &mut VersionsState, msg: VersionsMessage) {
    match msg {
        VersionsMessage::MoveUp => {
            super::move_up(&mut state.selected, state.tags.len());
        }
        VersionsMessage::MoveDown => {
            super::move_down(&mut state.selected, state.tags.len());
        }
        VersionsMessage::Refresh => {
            // Signal to reload tags — handled by caller
        }
        VersionsMessage::SetTags(tags) => {
            state.tags = tags;
            state.clamp_selected();
        }
    }
}

/// Render the versions screen.
pub fn render(state: &VersionsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Tag list
            Constraint::Length(3), // Detail of selected
        ])
        .split(area);

    render_tag_list(state, frame, chunks[0]);
    render_tag_detail(state, frame, chunks[1]);
}

/// Render the tag list.
fn render_tag_list(state: &VersionsState, frame: &mut Frame, area: Rect) {
    if state.tags.is_empty() {
        let block = Block::default().borders(Borders::ALL).title("Versions");
        let paragraph = Paragraph::new("No version tags found")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .tags
        .iter()
        .enumerate()
        .map(|(idx, tag)| {
            let style = super::list_item_style(idx == state.selected);

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", tag.name),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(if idx == state.selected {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
                Span::styled(
                    format!("({}) ", tag.date),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(tag.message.clone(), style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title("Versions");
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the detail area for the selected tag.
fn render_tag_detail(state: &VersionsState, frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Detail");

    match state.selected_tag() {
        Some(tag) => {
            let text = format!(" {} — {} — {}", tag.name, tag.date, tag.message);
            let paragraph = Paragraph::new(text)
                .block(block)
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(paragraph, area);
        }
        None => {
            let paragraph = Paragraph::new("No tag selected")
                .block(block)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(paragraph, area);
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_tags() -> Vec<VersionTag> {
        vec![
            VersionTag {
                name: "v1.0.0".to_string(),
                date: "2025-01-15".to_string(),
                message: "Initial release".to_string(),
            },
            VersionTag {
                name: "v1.1.0".to_string(),
                date: "2025-02-01".to_string(),
                message: "Added TUI".to_string(),
            },
            VersionTag {
                name: "v1.2.0".to_string(),
                date: "2025-03-10".to_string(),
                message: "Agent support".to_string(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = VersionsState::default();
        assert!(state.tags.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = VersionsState::default();
        state.tags = sample_tags();

        update(&mut state, VersionsMessage::MoveDown);
        assert_eq!(state.selected, 1);

        update(&mut state, VersionsMessage::MoveDown);
        assert_eq!(state.selected, 2);

        update(&mut state, VersionsMessage::MoveDown);
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = VersionsState::default();
        state.tags = sample_tags();

        update(&mut state, VersionsMessage::MoveUp);
        assert_eq!(state.selected, 2); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = VersionsState::default();
        update(&mut state, VersionsMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, VersionsMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn set_tags_populates() {
        let mut state = VersionsState::default();
        state.selected = 99;
        update(&mut state, VersionsMessage::SetTags(sample_tags()));
        assert_eq!(state.tags.len(), 3);
        assert_eq!(state.selected, 2); // clamped
    }

    #[test]
    fn set_tags_empty_clears() {
        let mut state = VersionsState::default();
        state.tags = sample_tags();
        state.selected = 2;
        update(&mut state, VersionsMessage::SetTags(vec![]));
        assert!(state.tags.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn selected_tag_returns_correct() {
        let mut state = VersionsState::default();
        state.tags = sample_tags();
        state.selected = 1;
        let tag = state.selected_tag().unwrap();
        assert_eq!(tag.name, "v1.1.0");
    }

    #[test]
    fn selected_tag_none_when_empty() {
        let state = VersionsState::default();
        assert!(state.selected_tag().is_none());
    }

    #[test]
    fn render_with_tags_does_not_panic() {
        let mut state = VersionsState::default();
        state.tags = sample_tags();
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
        assert!(text.contains("Versions"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = VersionsState::default();
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
    fn refresh_is_noop() {
        let mut state = VersionsState::default();
        state.tags = sample_tags();
        state.selected = 1;
        update(&mut state, VersionsMessage::Refresh);
        assert_eq!(state.selected, 1); // unchanged
        assert_eq!(state.tags.len(), 3);
    }
}
