use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// Which field in the launch dialog is focused.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DialogField {
    #[default]
    Agent,
    Model,
    Branch,
    LaunchButton,
    CancelButton,
}

/// Model options per agent.
fn model_options_for_agents() -> Vec<Vec<String>> {
    vec![
        // Claude Code
        vec![
            "opus".to_string(),
            "sonnet".to_string(),
            "haiku".to_string(),
        ],
        // Codex CLI
        vec!["o3".to_string(), "o4-mini".to_string()],
        // Gemini CLI
        vec![
            "gemini-2.5-pro".to_string(),
            "gemini-2.5-flash".to_string(),
        ],
    ]
}

/// State for the agent launch dialog.
#[derive(Debug)]
pub struct LaunchDialogState {
    pub agent_options: Vec<String>,
    pub selected_agent: usize,
    pub model_options: Vec<Vec<String>>,
    pub selected_model: usize,
    pub branch_input: String,
    pub focused_field: DialogField,
}

impl Default for LaunchDialogState {
    fn default() -> Self {
        Self {
            agent_options: vec![
                "Claude Code".to_string(),
                "Codex CLI".to_string(),
                "Gemini CLI".to_string(),
            ],
            selected_agent: 0,
            model_options: model_options_for_agents(),
            selected_model: 0,
            branch_input: String::new(),
            focused_field: DialogField::Agent,
        }
    }
}

impl LaunchDialogState {
    /// Cycle focus to the next dialog field.
    pub fn focus_next(&mut self) {
        self.focused_field = match self.focused_field {
            DialogField::Agent => DialogField::Model,
            DialogField::Model => DialogField::Branch,
            DialogField::Branch => DialogField::LaunchButton,
            DialogField::LaunchButton => DialogField::CancelButton,
            DialogField::CancelButton => DialogField::Agent,
        };
    }

    /// Cycle the selected agent option forward.
    pub fn next_agent(&mut self) {
        if !self.agent_options.is_empty() {
            self.selected_agent = (self.selected_agent + 1) % self.agent_options.len();
            // Reset model selection when agent changes
            self.selected_model = 0;
        }
    }

    /// Cycle the selected model option forward.
    pub fn next_model(&mut self) {
        if let Some(models) = self.model_options.get(self.selected_agent) {
            if !models.is_empty() {
                self.selected_model = (self.selected_model + 1) % models.len();
            }
        }
    }

    /// Get the currently selected agent option label.
    pub fn selected_agent_label(&self) -> &str {
        self.agent_options
            .get(self.selected_agent)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get the currently selected model label.
    pub fn selected_model_label(&self) -> &str {
        self.model_options
            .get(self.selected_agent)
            .and_then(|models| models.get(self.selected_model))
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get the currently selected model name, or `None` if no models available.
    pub fn selected_model_name(&self) -> Option<&str> {
        self.model_options
            .get(self.selected_agent)
            .and_then(|models| models.get(self.selected_model))
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }
}

/// Return a style with REVERSED modifier when focused, or the base color otherwise.
fn button_style(focused: bool, color: Color) -> Style {
    if focused {
        Style::new()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    }
}

/// Render the launch dialog as a centered modal.
///
/// The caller is responsible for centering the `area` if needed.
/// This function renders directly into the given area.
pub fn render(buf: &mut Buffer, area: Rect, state: &LaunchDialogState) {
    Clear.render(area, buf);

    let popup_area = area;

    let block = Block::default()
        .title(" Launch Agent ")
        .borders(Borders::ALL)
        .style(Style::new().bg(Color::Black));

    let inner = block.inner(popup_area);
    block.render(popup_area, buf);

    let rows = Layout::vertical([
        Constraint::Length(1), // Agent selector
        Constraint::Length(1), // Model selector
        Constraint::Length(1), // Branch input
        Constraint::Length(1), // Spacer
        Constraint::Length(1), // Buttons
    ])
    .split(inner);

    // Agent selector
    let agent_style = if state.focused_field == DialogField::Agent {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Paragraph::new(Line::from(vec![
        Span::styled("Agent:   ", Style::new().fg(Color::DarkGray)),
        Span::styled(
            format!("[{} \u{25bc}]", state.selected_agent_label()),
            agent_style,
        ),
    ]))
    .render(rows[0], buf);

    // Model selector
    let model_style = if state.focused_field == DialogField::Model {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Paragraph::new(Line::from(vec![
        Span::styled("Model:   ", Style::new().fg(Color::DarkGray)),
        Span::styled(
            format!("[{} \u{25bc}]", state.selected_model_label()),
            model_style,
        ),
    ]))
    .render(rows[1], buf);

    // Branch input
    let branch_style = if state.focused_field == DialogField::Branch {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let branch_display = if state.branch_input.is_empty() {
        "<branch name>"
    } else {
        &state.branch_input
    };
    Paragraph::new(Line::from(vec![
        Span::styled("Branch:  ", Style::new().fg(Color::DarkGray)),
        Span::styled(format!("[{}]", branch_display), branch_style),
    ]))
    .render(rows[2], buf);

    // Buttons
    Paragraph::new(Line::from(vec![
        Span::raw("       "),
        Span::styled(
            " Launch ",
            button_style(state.focused_field == DialogField::LaunchButton, Color::Green),
        ),
        Span::raw("  "),
        Span::styled(
            " Cancel ",
            button_style(state.focused_field == DialogField::CancelButton, Color::Red),
        ),
    ]))
    .render(rows[4], buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialog_field_cycling() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.focused_field, DialogField::Agent);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::Model);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::Branch);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::LaunchButton);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::CancelButton);
        state.focus_next();
        assert_eq!(state.focused_field, DialogField::Agent);
    }

    #[test]
    fn test_dialog_render_centered() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        let state = LaunchDialogState::default();
        render(&mut buf, Rect::new(0, 0, 100, 30), &state);
        let all_content: String = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        assert!(all_content.contains("Launch Agent"));
    }

    #[test]
    fn test_launch_dialog_default_state() {
        let state = LaunchDialogState::default();
        assert_eq!(state.agent_options.len(), 3);
        assert_eq!(state.selected_agent, 0);
        assert_eq!(state.selected_model, 0);
        assert!(state.branch_input.is_empty());
        assert_eq!(state.focused_field, DialogField::Agent);
        assert_eq!(state.selected_agent_label(), "Claude Code");
        assert_eq!(state.selected_model_label(), "opus");
    }

    #[test]
    fn test_next_agent_cycles() {
        let mut state = LaunchDialogState::default();
        assert_eq!(state.selected_agent_label(), "Claude Code");
        state.next_agent();
        assert_eq!(state.selected_agent_label(), "Codex CLI");
        state.next_agent();
        assert_eq!(state.selected_agent_label(), "Gemini CLI");
        state.next_agent();
        assert_eq!(state.selected_agent_label(), "Claude Code");
    }

    #[test]
    fn test_next_model_cycles() {
        let mut state = LaunchDialogState::default();
        // Claude models: opus, sonnet, haiku
        assert_eq!(state.selected_model_label(), "opus");
        state.next_model();
        assert_eq!(state.selected_model_label(), "sonnet");
        state.next_model();
        assert_eq!(state.selected_model_label(), "haiku");
        state.next_model();
        assert_eq!(state.selected_model_label(), "opus");
    }

    #[test]
    fn test_agent_change_resets_model() {
        let mut state = LaunchDialogState::default();
        state.next_model(); // sonnet
        assert_eq!(state.selected_model_label(), "sonnet");
        state.next_agent(); // Switch to Codex
        assert_eq!(state.selected_model, 0);
        assert_eq!(state.selected_model_label(), "o3");
    }

    #[test]
    fn test_selected_model_name() {
        let state = LaunchDialogState::default();
        assert_eq!(state.selected_model_name(), Some("opus"));
    }

    #[test]
    fn test_model_options_per_agent() {
        let mut state = LaunchDialogState::default();
        // Claude: 3 models
        assert_eq!(state.model_options[0].len(), 3);
        // Codex: 2 models
        state.next_agent();
        assert_eq!(state.selected_model_label(), "o3");
        state.next_model();
        assert_eq!(state.selected_model_label(), "o4-mini");
        // Gemini: 2 models
        state.next_agent();
        assert_eq!(state.selected_model_label(), "gemini-2.5-pro");
        state.next_model();
        assert_eq!(state.selected_model_label(), "gemini-2.5-flash");
    }
}
