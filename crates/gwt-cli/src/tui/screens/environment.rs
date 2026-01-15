//! Environment Variables Management Screen

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};

/// Environment variable item
#[derive(Debug, Clone)]
pub struct EnvItem {
    /// Variable key
    pub key: String,
    /// Variable value
    pub value: String,
    /// Is the value masked (for secrets)
    pub is_secret: bool,
}

/// Input field being edited
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditField {
    #[default]
    Key,
    Value,
}

/// Environment variables state
#[derive(Debug, Default)]
pub struct EnvironmentState {
    /// Environment variables
    pub variables: Vec<EnvItem>,
    /// Currently selected index
    pub selected: usize,
    /// Is in edit mode
    pub edit_mode: bool,
    /// Is creating new variable
    pub is_new: bool,
    /// Current edit field
    pub edit_field: EditField,
    /// Edit key value
    pub edit_key: String,
    /// Edit value
    pub edit_value: String,
    /// Cursor position
    pub cursor: usize,
    /// Error message
    pub error: Option<String>,
    /// Show values (toggle visibility)
    pub show_values: bool,
    /// Profile name (context)
    pub profile_name: Option<String>,
}

impl EnvironmentState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize with variables
    pub fn with_variables(mut self, variables: Vec<EnvItem>) -> Self {
        self.variables = variables;
        self
    }

    /// Set profile context
    pub fn with_profile(mut self, profile: &str) -> Self {
        self.profile_name = Some(profile.to_string());
        self
    }

    /// Get selected variable
    pub fn selected_variable(&self) -> Option<&EnvItem> {
        self.variables.get(self.selected)
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if !self.edit_mode && self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.edit_mode && self.selected < self.variables.len().saturating_sub(1) {
            self.selected += 1;
        }
    }

    /// Toggle value visibility
    pub fn toggle_visibility(&mut self) {
        self.show_values = !self.show_values;
    }

    /// Enter edit mode for new variable
    pub fn start_new(&mut self) {
        self.edit_mode = true;
        self.is_new = true;
        self.edit_field = EditField::Key;
        self.edit_key.clear();
        self.edit_value.clear();
        self.cursor = 0;
        self.error = None;
    }

    /// Enter edit mode for existing variable
    pub fn start_edit(&mut self) {
        let var_data = self
            .selected_variable()
            .map(|v| (v.key.clone(), v.value.clone()));
        if let Some((key, value)) = var_data {
            self.edit_mode = true;
            self.is_new = false;
            self.edit_field = EditField::Value;
            self.edit_key = key;
            self.edit_value = value.clone();
            self.cursor = value.len();
            self.error = None;
        }
    }

    /// Exit edit mode
    pub fn cancel_edit(&mut self) {
        self.edit_mode = false;
        self.is_new = false;
        self.edit_key.clear();
        self.edit_value.clear();
        self.cursor = 0;
    }

    /// Switch between key and value fields
    pub fn switch_field(&mut self) {
        if self.edit_mode && self.is_new {
            match self.edit_field {
                EditField::Key => {
                    self.edit_field = EditField::Value;
                    self.cursor = self.edit_value.len();
                }
                EditField::Value => {
                    self.edit_field = EditField::Key;
                    self.cursor = self.edit_key.len();
                }
            }
        }
    }

    /// Get current edit text reference
    fn current_text_mut(&mut self) -> &mut String {
        match self.edit_field {
            EditField::Key => &mut self.edit_key,
            EditField::Value => &mut self.edit_value,
        }
    }

    /// Insert character
    pub fn insert_char(&mut self, c: char) {
        if self.edit_mode {
            let cursor = self.cursor;
            match self.edit_field {
                EditField::Key => self.edit_key.insert(cursor, c),
                EditField::Value => self.edit_value.insert(cursor, c),
            }
            self.cursor += 1;
        }
    }

    /// Delete character
    pub fn delete_char(&mut self) {
        if self.edit_mode && self.cursor > 0 {
            self.cursor -= 1;
            let cursor = self.cursor;
            match self.edit_field {
                EditField::Key => {
                    self.edit_key.remove(cursor);
                }
                EditField::Value => {
                    self.edit_value.remove(cursor);
                }
            }
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        let max = match self.edit_field {
            EditField::Key => self.edit_key.len(),
            EditField::Value => self.edit_value.len(),
        };
        if self.cursor < max {
            self.cursor += 1;
        }
    }

    /// Validate edit
    pub fn validate(&self) -> Result<(String, String), &'static str> {
        let key = self.edit_key.trim();
        if key.is_empty() {
            return Err("Variable name cannot be empty");
        }
        if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err("Variable name can only contain letters, numbers, and underscores");
        }
        if self.is_new && self.variables.iter().any(|v| v.key == key) {
            return Err("Variable with this name already exists");
        }
        Ok((key.to_string(), self.edit_value.clone()))
    }

    /// Mark variable as secret
    pub fn toggle_secret(&mut self) {
        if let Some(var) = self.variables.get_mut(self.selected) {
            var.is_secret = !var.is_secret;
        }
    }
}

/// Render environment screen
pub fn render_environment(state: &EnvironmentState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Header
            Constraint::Min(5),    // Variables list
            Constraint::Length(4), // Edit area or actions
        ])
        .split(area);

    // Header
    let profile_info = state.profile_name.as_deref().unwrap_or("default");
    let visibility = if state.show_values {
        "visible"
    } else {
        "hidden"
    };
    let header = Paragraph::new(format!(
        "Environment Variables | Profile: {} | Values: {} ({} vars)",
        profile_info,
        visibility,
        state.variables.len()
    ))
    .style(Style::default().fg(Color::Cyan));
    frame.render_widget(header, chunks[0]);

    // Variables list
    if state.variables.is_empty() && !state.edit_mode {
        let empty = Paragraph::new("No environment variables. Press 'n' to add one.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
    } else {
        let items: Vec<ListItem> = state
            .variables
            .iter()
            .enumerate()
            .map(|(i, var)| {
                let value_display = format_value(var, state.show_values);

                let secret_marker = if var.is_secret { " [secret]" } else { "" };

                let line = Line::from(vec![
                    Span::styled(&var.key, Style::default().fg(Color::Yellow)),
                    Span::raw(" = "),
                    Span::styled(value_display, Style::default().fg(Color::Green)),
                    Span::styled(secret_marker, Style::default().fg(Color::Magenta)),
                ]);

                let style = if i == state.selected && !state.edit_mode {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default()
                };

                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, chunks[1]);
    }

    // Edit area or actions
    if state.edit_mode {
        render_edit_area(state, frame, chunks[2]);
    } else {
        let actions = "[n] New | [e] Edit | [d] Delete | [v] Toggle visibility | [s] Toggle secret | [Esc] Back";
        let footer = Paragraph::new(actions)
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::TOP));
        frame.render_widget(footer, chunks[2]);
    }

    // Show error
    if let Some(error) = &state.error {
        let error_area = Rect {
            x: area.x + 2,
            y: area.y + area.height - 1,
            width: area.width - 4,
            height: 1,
        };
        let error_msg = Paragraph::new(error.as_str()).style(Style::default().fg(Color::Red));
        frame.render_widget(error_msg, error_area);
    }
}

/// Render edit area
fn render_edit_area(state: &EnvironmentState, frame: &mut Frame, area: Rect) {
    let edit_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(if state.is_new {
            " New Variable "
        } else {
            " Edit Variable "
        });

    let inner = edit_block.inner(area);
    frame.render_widget(edit_block, area);

    let edit_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Key
            Constraint::Length(1), // Value
        ])
        .split(inner);

    // Key field
    let key_style = if state.edit_field == EditField::Key {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let key_text = if state.edit_key.is_empty() && state.is_new {
        "KEY (press Tab to switch)".to_string()
    } else {
        state.edit_key.clone()
    };
    let key_line = Line::from(vec![
        Span::styled("Key: ", Style::default().fg(Color::DarkGray)),
        Span::styled(key_text, key_style),
    ]);
    frame.render_widget(Paragraph::new(key_line), edit_chunks[0]);

    // Value field
    let value_style = if state.edit_field == EditField::Value {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let value_text = if state.edit_value.is_empty() {
        "(empty)".to_string()
    } else {
        state.edit_value.clone()
    };
    let value_line = Line::from(vec![
        Span::styled("Value: ", Style::default().fg(Color::DarkGray)),
        Span::styled(value_text, value_style),
    ]);
    frame.render_widget(Paragraph::new(value_line), edit_chunks[1]);

    // Set cursor position
    let (cursor_x, cursor_y) = match state.edit_field {
        EditField::Key => (inner.x + 5 + state.cursor as u16, edit_chunks[0].y),
        EditField::Value => (inner.x + 7 + state.cursor as u16, edit_chunks[1].y),
    };
    frame.set_cursor_position((cursor_x, cursor_y));
}

fn format_value(var: &EnvItem, show_values: bool) -> String {
    if show_values {
        var.value.clone()
    } else {
        "********".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_navigation() {
        let vars = vec![
            EnvItem {
                key: "FOO".to_string(),
                value: "bar".to_string(),
                is_secret: false,
            },
            EnvItem {
                key: "SECRET".to_string(),
                value: "hidden".to_string(),
                is_secret: true,
            },
        ];

        let mut state = EnvironmentState::new().with_variables(vars);
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_edit_mode() {
        let vars = vec![EnvItem {
            key: "FOO".to_string(),
            value: "bar".to_string(),
            is_secret: false,
        }];

        let mut state = EnvironmentState::new().with_variables(vars);

        state.start_new();
        assert!(state.edit_mode);
        assert!(state.is_new);
        assert_eq!(state.edit_field, EditField::Key);

        state.insert_char('T');
        state.insert_char('E');
        state.insert_char('S');
        state.insert_char('T');
        assert_eq!(state.edit_key, "TEST");

        state.switch_field();
        assert_eq!(state.edit_field, EditField::Value);

        state.insert_char('v');
        state.insert_char('a');
        state.insert_char('l');
        assert_eq!(state.edit_value, "val");

        state.cancel_edit();
        assert!(!state.edit_mode);
    }

    #[test]
    fn test_validation() {
        let mut state = EnvironmentState::new();
        state.edit_mode = true;
        state.is_new = true;

        state.edit_key = "".to_string();
        assert!(state.validate().is_err());

        state.edit_key = "VALID_KEY".to_string();
        state.edit_value = "value".to_string();
        assert!(state.validate().is_ok());

        state.edit_key = "invalid-key".to_string();
        assert!(state.validate().is_err());
    }

    #[test]
    fn test_empty_value_placeholder_not_saved() {
        let vars = vec![EnvItem {
            key: "EMPTY".to_string(),
            value: "".to_string(),
            is_secret: false,
        }];

        let mut state = EnvironmentState::new().with_variables(vars);
        state.start_edit();

        let (key, value) = state.validate().expect("validation should pass");
        assert_eq!(key, "EMPTY");
        assert!(value.is_empty());
    }

    #[test]
    fn test_hidden_values_are_masked() {
        let vars = vec![EnvItem {
            key: "TOKEN".to_string(),
            value: "secret-value".to_string(),
            is_secret: false,
        }];

        let state = EnvironmentState::new().with_variables(vars);
        let masked = format_value(&state.variables[0], false);

        assert_eq!(masked, "********");
    }
}
