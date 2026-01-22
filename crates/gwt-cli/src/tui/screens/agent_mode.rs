//! Agent Mode screen

use ratatui::{prelude::*, widgets::*};

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

#[derive(Debug, Clone)]
pub struct AgentTaskSummary {
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct AgentModeState {
    pub input: String,
    pub messages: Vec<AgentMessage>,
    pub tasks: Vec<AgentTaskSummary>,
    pub ai_ready: bool,
    pub ai_error: Option<String>,
    pub last_error: Option<String>,
    pub is_waiting: bool,
}

impl AgentModeState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            messages: Vec::new(),
            tasks: Vec::new(),
            ai_ready: false,
            ai_error: None,
            last_error: None,
            is_waiting: false,
        }
    }

    pub fn set_ai_status(&mut self, ready: bool, error: Option<String>) {
        self.ai_ready = ready;
        self.ai_error = error;
    }

    pub fn set_waiting(&mut self, waiting: bool) {
        self.is_waiting = waiting;
    }
}

pub fn render_agent_mode(
    state: &AgentModeState,
    frame: &mut Frame,
    area: Rect,
    status_message: Option<&str>,
) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let main_area = outer[0];
    let input_area = outer[1];

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(main_area);

    render_chat_panel(state, frame, main_chunks[0], status_message);
    render_task_panel(state, frame, main_chunks[1]);
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
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Chat ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    if let Some(status) = status_message {
        lines.push(Line::from(Span::styled(
            status,
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    if let Some(error) = state.last_error.as_deref() {
        lines.push(Line::from(Span::styled(
            error,
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
            message,
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            "Press Enter to configure AI settings.",
            Style::default().fg(Color::DarkGray),
        )));
    } else if state.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "Start by describing your task.",
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
                Span::raw(" "),
                Span::raw(msg.content.clone()),
            ]));
            lines.push(Line::from(""));
        }
        if state.is_waiting {
            lines.push(Line::from(Span::styled(
                "Thinking...",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn render_task_panel(state: &AgentModeState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Tasks ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if state.tasks.is_empty() {
        lines.push(Line::from(Span::styled(
            "No tasks yet.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for task in &state.tasks {
            lines.push(Line::from(vec![
                Span::styled(task.title.clone(), Style::default().fg(Color::White)),
                Span::raw(" "),
                Span::styled(
                    format!("[{}]", task.status),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_input_panel(state: &AgentModeState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Input ");
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

    let paragraph = Paragraph::new(text).style(style);
    frame.render_widget(paragraph, inner);
}
