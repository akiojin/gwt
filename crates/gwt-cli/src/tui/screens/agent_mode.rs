//! Agent Mode screen

use gwt_core::terminal::manager::PaneManager;
use ratatui::{prelude::*, style::Modifier, widgets::*};
use unicode_width::UnicodeWidthChar;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AgentRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub role: AgentRole,
    pub content: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AgentTaskSummary {
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct AgentModeState {
    pub input: String,
    pub input_cursor: usize,
    pub messages: Vec<AgentMessage>,
    #[allow(dead_code)]
    pub tasks: Vec<AgentTaskSummary>,
    pub ai_ready: bool,
    pub ai_error: Option<String>,
    pub last_error: Option<String>,
    pub is_waiting: bool,
    /// FR-124: Selected task index in the pane task list
    pub selected_task_index: usize,
}

impl AgentModeState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            input_cursor: 0,
            messages: Vec::new(),
            tasks: Vec::new(),
            ai_ready: false,
            ai_error: None,
            last_error: None,
            is_waiting: false,
            selected_task_index: 0,
        }
    }

    pub fn set_ai_status(&mut self, ready: bool, error: Option<String>) {
        self.ai_ready = ready;
        self.ai_error = error;
    }

    pub fn set_waiting(&mut self, waiting: bool) {
        self.is_waiting = waiting;
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_idx = char_to_byte_index(&self.input, self.input_cursor);
        self.input.insert(byte_idx, c);
        self.input_cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let start = char_to_byte_index(&self.input, self.input_cursor - 1);
        let end = char_to_byte_index(&self.input, self.input_cursor);
        self.input.replace_range(start..end, "");
        self.input_cursor -= 1;
    }

    pub fn cursor_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    pub fn cursor_right(&mut self) {
        let len = self.input.chars().count();
        if self.input_cursor < len {
            self.input_cursor += 1;
        }
    }
}

pub fn render_agent_mode(
    state: &AgentModeState,
    frame: &mut Frame,
    area: Rect,
    status_message: Option<&str>,
    pane_manager: &PaneManager,
) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(5)])
        .split(area);

    let main_area = outer[0];
    let input_area = outer[1];

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(main_area);

    render_chat_panel(state, frame, main_chunks[0], status_message);
    // FR-123: Render task panel from PaneManager
    render_task_panel(state, frame, main_chunks[1], pane_manager);
    render_input_panel(state, frame, input_area);
}

fn render_chat_panel(
    state: &AgentModeState,
    frame: &mut Frame,
    area: Rect,
    status_message: Option<&str>,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(" Chat ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(status) = status_message {
        lines.push(Line::from(Span::styled(
            status.to_string(),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    if let Some(error) = state.last_error.as_deref() {
        lines.push(Line::from(Span::styled(
            error.to_string(),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    if !state.ai_ready {
        let message = state
            .ai_error
            .as_deref()
            .unwrap_or("AI settings are required.");
        lines.push(Line::from(Span::styled(
            message.to_string(),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            "Press Enter to configure AI settings.".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else if state.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "Start by describing your task.".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for msg in &state.messages {
            let (label, color) = match msg.role {
                AgentRole::User => ("User", Color::Green),
                AgentRole::Assistant => ("Assistant", Color::Cyan),
                AgentRole::System => ("System", Color::Yellow),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}:", label), Style::default().fg(color)),
                Span::raw(" ".to_string()),
                Span::raw(msg.content.clone()),
            ]));
            lines.push(Line::from(""));
        }
        if state.is_waiting {
            lines.push(Line::from(Span::styled(
                "Thinking...".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    let wrapped_lines = wrap_lines(&lines, inner.width);
    let scroll = wrapped_lines.len().saturating_sub(inner.height as usize);
    let paragraph = Paragraph::new(wrapped_lines).scroll((scroll as u16, 0));
    frame.render_widget(paragraph, inner);
}

/// FR-123: Render task panel from PaneManager's running agents.
/// FR-124: Highlight selected task and active pane.
fn render_task_panel(
    state: &AgentModeState,
    frame: &mut Frame,
    area: Rect,
    pane_manager: &PaneManager,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .title(" Agents ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panes = pane_manager.panes();
    let mut lines: Vec<Line<'static>> = Vec::new();

    if panes.is_empty() {
        lines.push(Line::from(Span::styled(
            "No agents running.".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let active_idx = pane_manager.active_index();
        for (i, pane) in panes.iter().enumerate() {
            let is_active = i == active_idx;
            let is_selected = i == state.selected_task_index;

            let status_str = match pane.status() {
                gwt_core::terminal::pane::PaneStatus::Running => "Running",
                gwt_core::terminal::pane::PaneStatus::Completed(code) => {
                    if *code == 0 {
                        "Done"
                    } else {
                        "Failed"
                    }
                }
                gwt_core::terminal::pane::PaneStatus::Error(_) => "Error",
            };
            let elapsed = chrono::Utc::now()
                .signed_duration_since(pane.started_at())
                .num_seconds();
            let elapsed_str = if elapsed >= 3600 {
                format!("{}h{}m", elapsed / 3600, (elapsed % 3600) / 60)
            } else if elapsed >= 60 {
                format!("{}m{}s", elapsed / 60, elapsed % 60)
            } else {
                format!("{}s", elapsed)
            };

            let marker = if is_active { ">" } else { " " };
            let agent_color = pane.agent_color();

            let style = if is_selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };

            lines.push(Line::from(vec![
                Span::styled(format!("{} ", marker), style),
                Span::styled(
                    pane.agent_name().to_string(),
                    style.fg(agent_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" [{}]", status_str), style.fg(Color::Yellow)),
                Span::styled(format!(" {}", elapsed_str), style.fg(Color::DarkGray)),
            ]));

            // Show branch name on second line
            lines.push(Line::from(vec![
                Span::styled("  ".to_string(), style),
                Span::styled(pane.branch_name().to_string(), style.fg(Color::DarkGray)),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_input_panel(state: &AgentModeState, frame: &mut Frame, area: Rect) {
    let border_color = if state.ai_ready {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Input ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = if state.ai_ready {
        state.input.clone()
    } else {
        "AI settings required".to_string()
    };
    let style = if state.ai_ready {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let wrapped_lines = wrap_text_lines(&text, inner.width, style);
    let (cursor_line, cursor_col, total_lines) =
        cursor_position(&text, state.input_cursor, inner.width);
    let inner_height = inner.height as usize;
    let scroll = if total_lines > inner_height {
        cursor_line.saturating_sub(inner_height.saturating_sub(1))
    } else {
        0
    };
    let paragraph = Paragraph::new(wrapped_lines).scroll((scroll as u16, 0));
    frame.render_widget(paragraph, inner);

    if state.ai_ready && inner.width > 0 && inner.height > 0 {
        let visible_line = cursor_line.saturating_sub(scroll) as u16;
        if visible_line < inner.height {
            let max_x = inner.width.saturating_sub(1) as usize;
            let cursor_x = inner.x + cursor_col.min(max_x) as u16;
            let cursor_y = inner.y + visible_line;
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

fn wrap_lines(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let mut wrapped = Vec::new();
    for line in lines {
        let mut inner = wrap_spans_to_lines(&line.spans, width);
        wrapped.append(&mut inner);
    }
    if wrapped.is_empty() {
        wrapped.push(Line::from(""));
    }
    wrapped
}

fn wrap_text_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let spans = vec![Span::styled(text.to_string(), style)];
    wrap_spans_to_lines(&spans, width)
}

fn wrap_spans_to_lines(spans: &[Span<'static>], width: u16) -> Vec<Line<'static>> {
    let width = width as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut segments: Vec<(Style, String)> = Vec::new();
    let mut current_width = 0usize;

    let flush_line = |segments: &mut Vec<(Style, String)>, lines: &mut Vec<Line>| {
        if segments.is_empty() {
            lines.push(Line::from(""));
        } else {
            let spans: Vec<Span<'static>> = segments
                .drain(..)
                .map(|(style, text)| Span::styled(text, style))
                .collect();
            lines.push(Line::from(spans));
        }
    };

    for span in spans {
        let style = span.style;
        for ch in span.content.as_ref().chars() {
            if ch == '\n' {
                flush_line(&mut segments, &mut lines);
                current_width = 0;
                continue;
            }
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if width > 0 && current_width + ch_width > width {
                flush_line(&mut segments, &mut lines);
                current_width = 0;
            }
            push_styled_char(&mut segments, style, ch);
            current_width += ch_width;
        }
    }
    if !segments.is_empty() || lines.is_empty() {
        flush_line(&mut segments, &mut lines);
    }
    lines
}

fn push_styled_char(segments: &mut Vec<(Style, String)>, style: Style, ch: char) {
    if let Some((last_style, text)) = segments.last_mut() {
        if *last_style == style {
            text.push(ch);
            return;
        }
    }
    segments.push((style, ch.to_string()));
}

fn cursor_position(text: &str, cursor: usize, width: u16) -> (usize, usize, usize) {
    let cursor = cursor.min(text.chars().count());
    let width = width.max(1) as usize;
    let mut line = 0usize;
    let mut col = 0usize;
    let mut index = 0usize;
    let mut cursor_line = 0usize;
    let mut cursor_col = 0usize;

    for ch in text.chars() {
        if index == cursor {
            cursor_line = line;
            cursor_col = col;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
            index += 1;
            continue;
        }
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if col + ch_width > width {
            line += 1;
            col = 0;
        }
        col += ch_width;
        index += 1;
    }

    if cursor == index {
        cursor_line = line;
        cursor_col = col;
    }

    let total_lines = line + 1;
    (cursor_line, cursor_col, total_lines)
}

fn char_to_byte_index(text: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    match text.char_indices().nth(cursor) {
        Some((idx, _)) => idx,
        None => text.len(),
    }
}
