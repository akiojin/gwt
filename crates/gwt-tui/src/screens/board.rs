//! Shared Board management tab.

use gwt_core::coordination::{BoardEntry, BoardEntryKind, CoordinationSnapshot};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::theme;

#[derive(Debug, Clone)]
pub struct BoardState {
    pub(crate) snapshot: CoordinationSnapshot,
    pub(crate) selected_entry: usize,
    pub(crate) selected_card: usize,
    pub(crate) composer_open: bool,
    pub(crate) composer_kind: BoardEntryKind,
    pub(crate) composer_text: String,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            snapshot: CoordinationSnapshot::default(),
            selected_entry: 0,
            selected_card: 0,
            composer_open: true,
            composer_kind: BoardEntryKind::Request,
            composer_text: String::new(),
        }
    }
}

impl BoardState {
    pub fn selected_entry(&self) -> Option<&BoardEntry> {
        self.snapshot.board.entries.get(self.selected_entry)
    }

    fn clamp_selected(&mut self) {
        super::clamp_index(&mut self.selected_entry, self.snapshot.board.entries.len());
        super::clamp_index(&mut self.selected_card, self.snapshot.cards.cards.len());
    }
}

#[derive(Debug, Clone)]
pub enum BoardMessage {
    MoveUp,
    MoveDown,
    Refresh,
    SetSnapshot(CoordinationSnapshot),
    OpenComposer,
    CloseComposer,
    ComposerInput(char),
    ComposerBackspace,
    CycleComposerKind,
    SubmitComposer,
}

pub fn update(state: &mut BoardState, msg: BoardMessage) {
    match msg {
        BoardMessage::MoveUp => {
            super::move_up(
                &mut state.selected_entry,
                state.snapshot.board.entries.len(),
            );
        }
        BoardMessage::MoveDown => {
            super::move_down(
                &mut state.selected_entry,
                state.snapshot.board.entries.len(),
            );
        }
        BoardMessage::Refresh | BoardMessage::SubmitComposer => {}
        BoardMessage::SetSnapshot(snapshot) => {
            let previous_len = state.snapshot.board.entries.len();
            let followed_tail = previous_len > 0 && state.selected_entry + 1 == previous_len;
            state.snapshot = snapshot;
            if followed_tail && !state.snapshot.board.entries.is_empty() {
                state.selected_entry = state.snapshot.board.entries.len() - 1;
            }
            state.clamp_selected();
        }
        BoardMessage::OpenComposer => {
            state.composer_open = true;
        }
        BoardMessage::CloseComposer => {
            state.composer_open = false;
        }
        BoardMessage::ComposerInput(ch) => {
            state.composer_open = true;
            state.composer_text.push(ch);
        }
        BoardMessage::ComposerBackspace => {
            state.composer_open = true;
            state.composer_text.pop();
        }
        BoardMessage::CycleComposerKind => {
            state.composer_kind = next_kind(&state.composer_kind);
        }
    }
}

pub fn render(state: &BoardState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(7)])
        .split(area);

    let main = chunks[0];
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(main);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(columns[1]);

    render_timeline(state, frame, columns[0]);
    render_entry_detail(state, frame, right[0]);
    render_cards(state, frame, right[1]);

    render_composer(state, frame, chunks[1]);
}

fn render_timeline(state: &BoardState, frame: &mut Frame, area: Rect) {
    if state.snapshot.board.entries.is_empty() {
        let paragraph = Paragraph::new("No board entries yet")
            .block(Block::default().title("Timeline"))
            .style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .snapshot
        .board
        .entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let style = if idx == state.selected_entry {
                theme::style::selected_item()
            } else {
                theme::style::text()
            };
            let preview = entry.body.lines().next().unwrap_or("").trim().to_string();
            let line = Line::from(vec![
                super::selection_prefix(idx == state.selected_entry),
                Span::styled(
                    format!("[{}] ", entry.kind.as_str()),
                    Style::default().fg(kind_color(&entry.kind)),
                ),
                Span::styled(
                    format!("{} ", entry.author),
                    Style::default()
                        .fg(theme::color::ACTIVE)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(preview, style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected_entry));
    let list = List::new(items)
        .block(Block::default().title(format!("Timeline ({})", state.snapshot.board.entries.len())))
        .highlight_style(theme::style::active_item());
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_entry_detail(state: &BoardState, frame: &mut Frame, area: Rect) {
    let Some(entry) = state.selected_entry() else {
        let paragraph = Paragraph::new("Select a board entry to inspect the thread")
            .block(Block::default().title("Thread"))
            .style(theme::style::muted_text())
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
        return;
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("[{}]", entry.kind.as_str()),
                Style::default()
                    .fg(kind_color(&entry.kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                entry.author.clone(),
                Style::default().fg(theme::color::ACTIVE),
            ),
        ]),
        Line::from(format!(
            "State: {}",
            entry.state.as_deref().unwrap_or("n/a")
        )),
        Line::from(format!(
            "Created: {}",
            entry.created_at.format("%Y-%m-%d %H:%M:%S UTC")
        )),
    ];

    if !entry.related_topics.is_empty() {
        lines.push(Line::from(format!(
            "Topics: {}",
            entry.related_topics.join(", ")
        )));
    }
    if !entry.related_owners.is_empty() {
        lines.push(Line::from(format!(
            "Owners: {}",
            entry.related_owners.join(", ")
        )));
    }
    if let Some(parent) = entry.parent_id.as_deref() {
        lines.push(Line::from(format!("Parent: {parent}")));
    }
    lines.push(Line::default());
    for body_line in entry.body.lines() {
        lines.push(Line::from(body_line.to_string()));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().title("Thread"))
        .wrap(Wrap { trim: false })
        .style(theme::style::text());
    frame.render_widget(paragraph, area);
}

fn render_cards(state: &BoardState, frame: &mut Frame, area: Rect) {
    if state.snapshot.cards.cards.is_empty() {
        let paragraph = Paragraph::new("No agent cards yet")
            .block(Block::default().title("Cards"))
            .style(theme::style::muted_text())
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, area);
        return;
    }

    let mut lines = Vec::new();
    for (idx, card) in state.snapshot.cards.cards.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::default());
        }
        let name_style = if idx == state.selected_card {
            Style::default()
                .fg(theme::color::ACTIVE)
                .add_modifier(Modifier::BOLD)
        } else {
            theme::style::text()
        };
        lines.push(Line::from(vec![
            Span::styled(card.agent_id.clone(), name_style),
            Span::raw(" "),
            Span::styled(
                format!("[{}]", card.status.as_deref().unwrap_or("unknown")),
                Style::default().fg(theme::color::FOCUS),
            ),
        ]));
        lines.push(Line::from(format!("Branch: {}", card.branch)));
        if let Some(focus) = card.current_focus.as_deref() {
            lines.push(Line::from(format!("Focus: {focus}")));
        }
        if let Some(next) = card.next_action.as_deref() {
            lines.push(Line::from(format!("Next: {next}")));
        }
        if let Some(reason) = card.blocked_reason.as_deref() {
            lines.push(Line::from(format!("Blocked: {reason}")));
        }
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().title(format!("Cards ({})", state.snapshot.cards.cards.len())))
        .wrap(Wrap { trim: false })
        .style(theme::style::text());
    frame.render_widget(paragraph, area);
}

fn render_composer(state: &BoardState, frame: &mut Frame, area: Rect) {
    let title = format!("Input [{}]", state.composer_kind.as_str());
    let block = Block::default().title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let raw_lines: Vec<&str> = if state.composer_text.is_empty() {
        vec![""]
    } else {
        state.composer_text.split('\n').collect()
    };
    let visible_start = raw_lines.len().saturating_sub(inner.height as usize);
    let visible_lines = &raw_lines[visible_start..];
    let mut lines = Vec::with_capacity(inner.height as usize);
    let pad = inner.height as usize - visible_lines.len();
    lines.extend(std::iter::repeat_with(Line::default).take(pad));

    for (offset, line) in visible_lines.iter().enumerate() {
        let overall_idx = visible_start + offset;
        let prefix = if overall_idx == 0 { "> " } else { "  " };
        let mut spans = vec![Span::styled(prefix, theme::style::active_item())];
        if state.composer_text.is_empty() && overall_idx == 0 && !state.composer_open {
            spans.push(Span::styled("Type to post", theme::style::muted_text()));
        } else {
            spans.push(Span::styled((*line).to_string(), theme::style::text()));
        }
        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(Text::from(lines)).style(theme::style::text());
    frame.render_widget(paragraph, inner);

    if state.composer_open {
        let last_line = raw_lines.last().copied().unwrap_or("");
        let last_prefix = if raw_lines.len() == 1 { "> " } else { "  " };
        let cursor_x = inner.x
            + (UnicodeWidthStr::width(last_prefix) + UnicodeWidthStr::width(last_line))
                .min(inner.width.saturating_sub(1) as usize) as u16;
        let cursor_y = inner.y + inner.height.saturating_sub(1);
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn next_kind(kind: &BoardEntryKind) -> BoardEntryKind {
    match kind {
        BoardEntryKind::Request => BoardEntryKind::Status,
        BoardEntryKind::Status => BoardEntryKind::Question,
        BoardEntryKind::Question => BoardEntryKind::Blocked,
        BoardEntryKind::Blocked => BoardEntryKind::Handoff,
        BoardEntryKind::Handoff => BoardEntryKind::Decision,
        BoardEntryKind::Decision => BoardEntryKind::Next,
        BoardEntryKind::Next => BoardEntryKind::Impact,
        BoardEntryKind::Impact => BoardEntryKind::Claim,
        BoardEntryKind::Claim => BoardEntryKind::Request,
    }
}

fn kind_color(kind: &BoardEntryKind) -> Color {
    match kind {
        BoardEntryKind::Request => theme::color::ACTIVE,
        BoardEntryKind::Status => theme::color::SUCCESS,
        BoardEntryKind::Question => theme::color::FOCUS,
        BoardEntryKind::Blocked => theme::color::ERROR,
        BoardEntryKind::Handoff => theme::color::ACCENT,
        BoardEntryKind::Decision => Color::Cyan,
        BoardEntryKind::Next => Color::Yellow,
        BoardEntryKind::Impact => Color::Magenta,
        BoardEntryKind::Claim => Color::Green,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::coordination::{AgentCard, AgentCardsProjection, AuthorKind, BoardProjection};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use ratatui::Terminal;

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect()
    }

    fn buffer_line(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    fn render_buffer(state: &BoardState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(state, frame, frame.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn render_cursor_position(state: &BoardState, width: u16, height: u16) -> Position {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(state, frame, frame.area()))
            .unwrap();
        terminal.get_cursor_position().unwrap()
    }

    fn sample_snapshot() -> CoordinationSnapshot {
        CoordinationSnapshot {
            board: BoardProjection {
                entries: vec![BoardEntry::new(
                    AuthorKind::User,
                    "user",
                    BoardEntryKind::Request,
                    "Need a shared board",
                    None,
                    None,
                    vec!["coordination".into()],
                    vec!["1974".into()],
                )],
                updated_at: chrono::Utc::now(),
            },
            cards: AgentCardsProjection {
                cards: vec![AgentCard {
                    agent_id: "Codex".into(),
                    session_id: Some("sess-1".into()),
                    branch: "feature/demo".into(),
                    role: None,
                    responsibility: None,
                    status: Some("running".into()),
                    current_focus: Some("Board tab".into()),
                    next_action: Some("Add watcher".into()),
                    blocked_reason: None,
                    related_topics: vec![],
                    related_owners: vec![],
                    working_scope: None,
                    handoff_target: None,
                    updated_at: chrono::Utc::now(),
                }],
                updated_at: chrono::Utc::now(),
            },
        }
    }

    #[test]
    fn set_snapshot_tracks_tail_and_clamps_selection() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::SetSnapshot(sample_snapshot()));
        assert_eq!(state.selected_entry, 0);

        let mut snapshot = sample_snapshot();
        snapshot.board.entries.push(BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "Working",
            Some("running".into()),
            None,
            vec![],
            vec![],
        ));
        update(&mut state, BoardMessage::SetSnapshot(snapshot));

        assert_eq!(state.selected_entry, 1);
        assert_eq!(
            state.selected_entry().map(|entry| entry.body.as_str()),
            Some("Working")
        );
    }

    #[test]
    fn composer_messages_toggle_and_edit_text() {
        let mut state = BoardState::default();

        update(&mut state, BoardMessage::OpenComposer);
        update(&mut state, BoardMessage::ComposerInput('h'));
        update(&mut state, BoardMessage::ComposerInput('i'));
        update(&mut state, BoardMessage::ComposerBackspace);
        update(&mut state, BoardMessage::CycleComposerKind);
        update(&mut state, BoardMessage::CloseComposer);

        assert!(!state.composer_open);
        assert_eq!(state.composer_text, "h");
        assert_eq!(state.composer_kind, BoardEntryKind::Status);
    }

    #[test]
    fn render_shows_input_footer_even_when_not_editing() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::CloseComposer);

        let text = buffer_text(&render_buffer(&state, 100, 24));

        assert!(text.contains("Input [request]"));
        assert!(text.contains("Type to post"));
    }

    #[test]
    fn render_places_prompt_on_bottom_row() {
        let state = BoardState::default();

        let buf = render_buffer(&state, 100, 24);
        let line = buffer_line(&buf, 23);

        assert!(line.contains("> "));
    }

    #[test]
    fn render_shows_active_input_placeholder_when_editing() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::OpenComposer);

        let text = buffer_text(&render_buffer(&state, 100, 24));

        assert!(text.contains("Input [request]"));
        assert!(text.contains("> "));
    }

    #[test]
    fn render_places_cursor_after_prompt_when_editing() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::ComposerInput('h'));
        update(&mut state, BoardMessage::ComposerInput('i'));

        let position = render_cursor_position(&state, 100, 24);

        assert_eq!(position, Position::new(4, 23));
    }
}
