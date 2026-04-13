//! Shared Board management tab.

use gwt_core::coordination::{BoardEntry, BoardEntryKind, CoordinationSnapshot};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme;

const COMPOSER_HEIGHT: u16 = 7;
const DETAIL_HEIGHT: u16 = 9;

#[derive(Debug, Clone)]
pub struct BoardState {
    pub(crate) snapshot: CoordinationSnapshot,
    pub(crate) selected_entry: usize,
    pub(crate) detail_open: bool,
    pub(crate) composer_open: bool,
    pub(crate) composer_kind: BoardEntryKind,
    pub(crate) composer_text: String,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            snapshot: CoordinationSnapshot::default(),
            selected_entry: 0,
            detail_open: false,
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
        if self.selected_entry().is_none() {
            self.detail_open = false;
        }
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
    OpenDetail,
    CloseDetail,
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
        BoardMessage::OpenDetail => {
            if state.selected_entry().is_some() {
                state.detail_open = true;
            }
        }
        BoardMessage::CloseDetail => {
            state.detail_open = false;
        }
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
    if state.detail_open {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(DETAIL_HEIGHT),
                Constraint::Length(COMPOSER_HEIGHT),
            ])
            .split(area);
        render_timeline(state, frame, chunks[0]);
        render_detail_bottom_sheet(state, frame, chunks[1]);
        render_composer(state, frame, chunks[2]);
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(COMPOSER_HEIGHT)])
            .split(area);
        render_timeline(state, frame, chunks[0]);
        render_composer(state, frame, chunks[1]);
    }
}

fn render_timeline(state: &BoardState, frame: &mut Frame, area: Rect) {
    let block = Block::default().title(format!("Chat ({})", state.snapshot.board.entries.len()));
    let inner = block.inner(area);

    if state.snapshot.board.entries.is_empty() {
        let paragraph = Paragraph::new("No chat messages yet")
            .block(block)
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
        .map(|(idx, entry)| build_chat_item(entry, idx == state.selected_entry, inner.width))
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected_entry));
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::style::active_item());
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn build_chat_item(entry: &BoardEntry, is_selected: bool, width: u16) -> ListItem<'_> {
    let chat_width = width.saturating_sub(2) as usize;
    let bubble_width = if chat_width < 18 {
        chat_width.max(1)
    } else {
        (chat_width * 3 / 5).max(18)
    };
    let lines = match entry.author_kind {
        gwt_core::coordination::AuthorKind::System => {
            let event_lines =
                wrap_text_lines(&format!("{} · {}", entry.author, entry.body), bubble_width);
            event_lines
                .into_iter()
                .map(|line| {
                    let padded = pad_to_alignment(&line, chat_width, ChatAlignment::Center);
                    Line::from(Span::styled(padded, message_body_style(entry, is_selected)))
                })
                .collect()
        }
        gwt_core::coordination::AuthorKind::User | gwt_core::coordination::AuthorKind::Agent => {
            let align = if matches!(entry.author_kind, gwt_core::coordination::AuthorKind::User) {
                ChatAlignment::Left
            } else {
                ChatAlignment::Right
            };
            let mut lines = Vec::new();
            let header = pad_to_alignment(&entry.author, chat_width, align);
            lines.push(Line::from(Span::styled(
                header,
                message_header_style(entry, is_selected),
            )));
            for body_line in wrap_text_lines(&entry.body, bubble_width) {
                let padded = pad_to_alignment(&body_line, chat_width, align);
                lines.push(Line::from(Span::styled(
                    padded,
                    message_body_style(entry, is_selected),
                )));
            }
            lines
        }
    };

    ListItem::new(Text::from(lines))
}

fn render_detail_bottom_sheet(state: &BoardState, frame: &mut Frame, area: Rect) {
    let Some(entry) = state.selected_entry() else {
        return;
    };

    let mut lines = vec![
        Line::from(format!("Author: {}", entry.author)),
        Line::from(format!(
            "Author Kind: {}",
            author_kind_label(&entry.author_kind)
        )),
        Line::from(format!("Kind: {}", entry.kind.as_str())),
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
    if let Some(parent_id) = entry.parent_id.as_deref() {
        lines.push(Line::from(format!("Parent: {parent_id}")));
    }
    lines.push(Line::default());
    for body_line in entry.body.lines() {
        lines.push(Line::from(body_line.to_string()));
    }

    let paragraph = Paragraph::new(Text::from(lines))
        .block(Block::default().title("Details"))
        .wrap(Wrap { trim: false })
        .style(theme::style::text());
    frame.render_widget(paragraph, area);
}

#[derive(Clone, Copy)]
enum ChatAlignment {
    Left,
    Right,
    Center,
}

fn pad_to_alignment(line: &str, width: usize, align: ChatAlignment) -> String {
    let line_width = UnicodeWidthStr::width(line);
    let offset = match align {
        ChatAlignment::Left => 1,
        ChatAlignment::Right => width.saturating_sub(line_width).saturating_sub(2),
        ChatAlignment::Center => width.saturating_sub(line_width) / 2,
    };
    format!("{}{}", " ".repeat(offset), line)
}

fn wrap_text_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut result = Vec::new();

    for raw_line in text.lines() {
        if raw_line.is_empty() {
            result.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };

            if UnicodeWidthStr::width(candidate.as_str()) <= width {
                current = candidate;
                continue;
            }

            if !current.is_empty() {
                result.push(current);
                current = String::new();
            }

            if UnicodeWidthStr::width(word) <= width {
                current = word.to_string();
            } else {
                result.extend(hard_wrap_word(word, width));
            }
        }

        if !current.is_empty() {
            result.push(current);
        }
    }

    if result.is_empty() {
        result.push(String::new());
    }

    result
}

fn hard_wrap_word(word: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in word.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > width && !current.is_empty() {
            result.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

fn message_header_style(entry: &BoardEntry, is_selected: bool) -> Style {
    let base = match entry.author_kind {
        gwt_core::coordination::AuthorKind::User => Style::default()
            .fg(theme::color::ACTIVE)
            .add_modifier(Modifier::BOLD),
        gwt_core::coordination::AuthorKind::Agent => Style::default()
            .fg(theme::color::ACCENT)
            .add_modifier(Modifier::BOLD),
        gwt_core::coordination::AuthorKind::System => Style::default()
            .fg(message_kind_color(&entry.kind))
            .add_modifier(Modifier::BOLD),
    };
    if is_selected {
        base.add_modifier(Modifier::UNDERLINED)
    } else {
        base
    }
}

fn message_body_style(entry: &BoardEntry, is_selected: bool) -> Style {
    let base = match entry.author_kind {
        gwt_core::coordination::AuthorKind::System => theme::style::muted_text(),
        _ => theme::style::text(),
    };
    if is_selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

fn author_kind_label(kind: &gwt_core::coordination::AuthorKind) -> &'static str {
    match kind {
        gwt_core::coordination::AuthorKind::User => "user",
        gwt_core::coordination::AuthorKind::Agent => "agent",
        gwt_core::coordination::AuthorKind::System => "system",
    }
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

fn message_kind_color(kind: &BoardEntryKind) -> Color {
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
    use gwt_core::coordination::{AgentCardsProjection, AuthorKind, BoardProjection};
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

    fn find_line_containing(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, String)> {
        (0..buf.area.height).find_map(|y| {
            let line = buffer_line(buf, y);
            line.contains(needle).then_some((y, line))
        })
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
        let first = BoardEntry::new(
            AuthorKind::User,
            "user",
            BoardEntryKind::Request,
            "Need a shared board",
            None,
            None,
            vec!["coordination".into()],
            vec!["1974".into()],
        );
        let second = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "Working on Board tab",
            Some("running".into()),
            Some(first.id.clone()),
            vec!["coordination".into()],
            vec!["1974".into()],
        );
        let third = BoardEntry::new(
            AuthorKind::System,
            "System",
            BoardEntryKind::Status,
            "Session resumed",
            None,
            None,
            vec![],
            vec![],
        );
        CoordinationSnapshot {
            board: BoardProjection {
                entries: vec![first, second, third],
                updated_at: chrono::Utc::now(),
            },
            cards: AgentCardsProjection::default(),
        }
    }

    #[test]
    fn set_snapshot_tracks_tail_and_clamps_selection() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::SetSnapshot(sample_snapshot()));
        state.selected_entry = 2;

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

        assert_eq!(state.selected_entry, 3);
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

    #[test]
    fn render_shows_single_chat_timeline_without_cards_or_thread_panes() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::SetSnapshot(sample_snapshot()));

        let text = buffer_text(&render_buffer(&state, 100, 24));

        assert!(text.contains("Chat (3)"));
        assert!(text.contains("Need a shared board"));
        assert!(text.contains("Working on Board tab"));
        assert!(text.contains("Session resumed"));
        assert!(text.contains("user"));
        assert!(text.contains("Codex"));
        assert!(!text.contains("state: running"));
        assert!(!text.contains("No agent cards yet"));
    }

    #[test]
    fn render_offsets_user_left_and_agent_right_for_chat_layout() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::SetSnapshot(sample_snapshot()));

        let buf = render_buffer(&state, 100, 24);
        let (_, user_line) = find_line_containing(&buf, "Need a shared board").unwrap();
        let (_, agent_line) = find_line_containing(&buf, "Working on Board tab").unwrap();

        assert!(user_line.find("Need a shared board").unwrap() < 12);
        assert!(agent_line.find("Working on Board tab").unwrap() > 32);
    }

    #[test]
    fn render_centers_system_event_rows() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::SetSnapshot(sample_snapshot()));

        let buf = render_buffer(&state, 100, 24);
        let (_, system_line) = find_line_containing(&buf, "Session resumed").unwrap();
        let offset = system_line.find("Session resumed").unwrap();

        assert!(
            offset > 24,
            "system line was not centered enough: {system_line:?}"
        );
        assert!(
            offset < 60,
            "system line drifted too far right: {system_line:?}"
        );
    }

    #[test]
    fn render_detail_bottom_sheet_shows_selected_message_metadata() {
        let mut state = BoardState::default();
        update(&mut state, BoardMessage::SetSnapshot(sample_snapshot()));
        state.selected_entry = 1;
        update(&mut state, BoardMessage::OpenDetail);

        let text = buffer_text(&render_buffer(&state, 100, 30));

        assert!(text.contains("Details"));
        assert!(text.contains("Author: Codex"));
        assert!(text.contains("Author Kind: agent"));
        assert!(text.contains("Kind: status"));
        assert!(text.contains("State: running"));
        assert!(text.contains("Topics: coordination"));
        assert!(text.contains("Owners: 1974"));
        assert!(text.contains("Parent:"));
        assert!(text.contains("Working on Board tab"));
    }
}
