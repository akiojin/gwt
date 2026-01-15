//! Environment Variables Management Screen

#![allow(dead_code)]

use ratatui::{prelude::*, widgets::*};
use std::collections::HashMap;

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

/// OS environment variable item
#[derive(Debug, Clone)]
pub struct OsEnvItem {
    /// Variable key
    pub key: String,
    /// Variable value
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvDisplayKind {
    OsOnly,
    Added,
    Overridden,
}

#[derive(Debug, Clone)]
struct DisplayEnvItem {
    key: String,
    value: String,
    kind: EnvDisplayKind,
    profile_index: Option<usize>,
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
    /// OS environment variables
    pub os_variables: Vec<OsEnvItem>,
    /// Currently selected index
    pub selected: usize,
    /// Scroll offset for large lists
    scroll_offset: usize,
    /// Cached viewport height for scroll calculations
    viewport_height: usize,
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
        self.clamp_selection();
        self
    }

    /// Initialize with OS variables
    pub fn with_os_variables(mut self, variables: Vec<OsEnvItem>) -> Self {
        self.os_variables = variables;
        self.clamp_selection();
        self
    }

    /// Set profile context
    pub fn with_profile(mut self, profile: &str) -> Self {
        self.profile_name = Some(profile.to_string());
        self
    }

    /// Get selected variable
    pub fn selected_variable(&self) -> Option<&EnvItem> {
        self.selected_profile_index()
            .and_then(|index| self.variables.get(index))
    }

    pub fn selected_profile_index(&self) -> Option<usize> {
        self.selected_display_item().and_then(|item| item.profile_index)
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if self.edit_mode {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
            self.ensure_visible();
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.edit_mode {
            return;
        }
        let total = self.display_len();
        if total == 0 {
            self.selected = 0;
            return;
        }
        if self.selected + 1 < total {
            self.selected += 1;
            self.ensure_visible();
        }
    }

    pub fn page_down(&mut self) {
        if self.edit_mode {
            return;
        }
        let total = self.display_len();
        if total == 0 {
            return;
        }
        let step = self.viewport_height.max(1);
        self.selected = (self.selected + step).min(total - 1);
        self.ensure_visible();
    }

    pub fn page_up(&mut self) {
        if self.edit_mode {
            return;
        }
        let step = self.viewport_height.max(1);
        self.selected = self.selected.saturating_sub(step);
        self.ensure_visible();
    }

    pub fn go_home(&mut self) {
        if self.edit_mode {
            return;
        }
        self.selected = 0;
        self.scroll_offset = 0;
    }

    pub fn go_end(&mut self) {
        if self.edit_mode {
            return;
        }
        let total = self.display_len();
        if total == 0 {
            return;
        }
        self.selected = total - 1;
        if self.viewport_height > 0 {
            self.scroll_offset = self.selected.saturating_sub(self.viewport_height - 1);
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

    pub fn start_edit_selected(&mut self) {
        let selected = match self.selected_display_item() {
            Some(item) => item,
            None => return,
        };

        if let Some(index) = selected.profile_index {
            self.start_edit_at(index);
        } else {
            self.start_override(selected.key, selected.value);
        }
    }

    /// Enter edit mode for existing variable
    pub fn start_edit(&mut self) {
        if let Some(index) = self.selected_profile_index() {
            self.start_edit_at(index);
        }
    }

    fn start_edit_at(&mut self, index: usize) {
        let var = match self.variables.get(index) {
            Some(var) => var,
            None => return,
        };
        self.edit_mode = true;
        self.is_new = false;
        self.edit_field = EditField::Value;
        self.edit_key = var.key.clone();
        self.edit_value = var.value.clone();
        self.cursor = self.edit_value.len();
        self.error = None;
    }

    fn start_override(&mut self, key: String, value: String) {
        self.edit_mode = true;
        self.is_new = true;
        self.edit_field = EditField::Value;
        self.edit_key = key;
        self.edit_value = value;
        self.cursor = self.edit_value.len();
        self.error = None;
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
        if let Some(index) = self.selected_profile_index() {
            if let Some(var) = self.variables.get_mut(index) {
                var.is_secret = !var.is_secret;
            }
        }
    }

    pub fn set_viewport(&mut self, height: usize) {
        self.viewport_height = height;
        self.ensure_visible();
    }

    pub fn refresh_selection(&mut self) {
        self.ensure_visible();
    }

    fn display_len(&self) -> usize {
        let mut keys: HashMap<&str, ()> = HashMap::new();
        for var in &self.os_variables {
            keys.insert(var.key.as_str(), ());
        }
        for var in &self.variables {
            keys.insert(var.key.as_str(), ());
        }
        keys.len()
    }

    fn selected_display_item(&self) -> Option<DisplayEnvItem> {
        let items = self.display_items();
        items.get(self.selected).cloned()
    }

    fn display_items(&self) -> Vec<DisplayEnvItem> {
        let mut os_map: HashMap<String, String> = HashMap::new();
        for var in &self.os_variables {
            os_map.insert(var.key.clone(), var.value.clone());
        }
        let mut profile_map: HashMap<String, (usize, String)> = HashMap::new();
        for (index, var) in self.variables.iter().enumerate() {
            profile_map.insert(var.key.clone(), (index, var.value.clone()));
        }

        let mut keys: Vec<String> = os_map.keys().cloned().collect();
        for key in profile_map.keys() {
            if !os_map.contains_key(key) {
                keys.push(key.clone());
            }
        }
        keys.sort();

        keys.into_iter()
            .map(|key| match (profile_map.get(&key), os_map.get(&key)) {
                (Some((index, profile_value)), Some(_os_value)) => DisplayEnvItem {
                    key,
                    value: profile_value.clone(),
                    kind: EnvDisplayKind::Overridden,
                    profile_index: Some(*index),
                },
                (Some((index, profile_value)), None) => DisplayEnvItem {
                    key,
                    value: profile_value.clone(),
                    kind: EnvDisplayKind::Added,
                    profile_index: Some(*index),
                },
                (None, Some(os_value)) => DisplayEnvItem {
                    key,
                    value: os_value.clone(),
                    kind: EnvDisplayKind::OsOnly,
                    profile_index: None,
                },
                (None, None) => DisplayEnvItem {
                    key,
                    value: String::new(),
                    kind: EnvDisplayKind::OsOnly,
                    profile_index: None,
                },
            })
            .collect()
    }

    fn visible_range(&self, total: usize) -> (usize, usize) {
        let height = self.viewport_height.max(1);
        let start = self.scroll_offset.min(total);
        let end = (start + height).min(total);
        (start, end)
    }

    fn ensure_visible(&mut self) {
        let total = self.display_len();
        if total == 0 {
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }
        if self.selected >= total {
            self.selected = total - 1;
        }
        if self.viewport_height == 0 {
            return;
        }
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.viewport_height {
            self.scroll_offset = self.selected + 1 - self.viewport_height;
        }
    }

    fn clamp_selection(&mut self) {
        self.ensure_visible();
    }
}

/// Collect OS environment variables as a sorted list.
pub fn collect_os_env() -> Vec<OsEnvItem> {
    let mut vars: Vec<OsEnvItem> = std::env::vars()
        .map(|(key, value)| OsEnvItem { key, value })
        .collect();
    vars.sort_by(|a, b| a.key.cmp(&b.key));
    vars
}

/// Render environment screen
pub fn render_environment(state: &mut EnvironmentState, frame: &mut Frame, area: Rect) {
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
    let total_vars = state.display_len();
    let header = Paragraph::new(format!(
        "Environment Variables | Profile: {} | Values: {} ({} vars)",
        profile_info,
        visibility,
        total_vars
    ))
    .style(Style::default().fg(Color::Cyan));
    frame.render_widget(header, chunks[0]);

    // Variables list
    let list_height = chunks[1].height as usize;
    state.set_viewport(list_height);
    let display_items = state.display_items();
    let total = display_items.len();

    if total == 0 && !state.edit_mode {
        let empty = Paragraph::new("No environment variables. Press 'n' to add one.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, chunks[1]);
    } else {
        let (start, end) = state.visible_range(total);
        let items: Vec<ListItem> = display_items[start..end]
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let absolute_index = start + i;
                let value_display = format_display_value(&item.value, state.show_values);
                let key_style = match item.kind {
                    EnvDisplayKind::Overridden => Style::default().fg(Color::Yellow),
                    EnvDisplayKind::Added => Style::default().fg(Color::Green),
                    EnvDisplayKind::OsOnly => Style::default(),
                };

                let line = Line::from(vec![
                    Span::styled(&item.key, key_style),
                    Span::raw(" = "),
                    Span::raw(value_display),
                ]);

                let style = if absolute_index == state.selected && !state.edit_mode {
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
        let actions = "[Enter/e] Edit | [n] New | [d] Delete | [v] Toggle visibility | [Esc] Back";
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

// OS environment list is merged into the main environment screen.

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

fn format_display_value(value: &str, show_values: bool) -> String {
    if show_values {
        value.to_string()
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
        let masked = format_display_value(&state.variables[0].value, false);

        assert_eq!(masked, "********");
    }

    #[test]
    fn test_collect_os_env_includes_added_var() {
        let key = "GWT_TEST_OS_ENV";
        let prev = std::env::var_os(key);
        std::env::set_var(key, "1");

        let vars = collect_os_env();
        let found = vars.iter().any(|var| var.key == key && var.value == "1");
        assert!(found);

        match prev {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn test_display_items_classify_kinds() {
        let os_vars = vec![
            OsEnvItem {
                key: "HOME".to_string(),
                value: "/tmp".to_string(),
            },
            OsEnvItem {
                key: "TOKEN".to_string(),
                value: "os-value".to_string(),
            },
        ];
        let profile_vars = vec![
            EnvItem {
                key: "TOKEN".to_string(),
                value: "override".to_string(),
                is_secret: false,
            },
            EnvItem {
                key: "NEW".to_string(),
                value: "added".to_string(),
                is_secret: false,
            },
        ];

        let state = EnvironmentState::new()
            .with_os_variables(os_vars)
            .with_variables(profile_vars);
        let items = state.display_items();

        let home = items.iter().find(|item| item.key == "HOME").unwrap();
        assert_eq!(home.kind, EnvDisplayKind::OsOnly);
        assert_eq!(home.value, "/tmp");
        assert!(home.profile_index.is_none());

        let token = items.iter().find(|item| item.key == "TOKEN").unwrap();
        assert_eq!(token.kind, EnvDisplayKind::Overridden);
        assert_eq!(token.value, "override");
        assert!(token.profile_index.is_some());

        let added = items.iter().find(|item| item.key == "NEW").unwrap();
        assert_eq!(added.kind, EnvDisplayKind::Added);
        assert_eq!(added.value, "added");
        assert!(added.profile_index.is_some());
    }

    #[test]
    fn test_env_scroll_offset_updates() {
        let os_vars = (0..10)
            .map(|i| OsEnvItem {
                key: format!("KEY{:02}", i),
                value: "value".to_string(),
            })
            .collect();

        let mut state = EnvironmentState::new().with_os_variables(os_vars);
        state.set_viewport(3);

        state.select_next();
        state.select_next();
        state.select_next();
        assert_eq!(state.selected, 3);
        assert_eq!(state.scroll_offset, 1);

        state.page_down();
        assert_eq!(state.selected, 6);
        assert_eq!(state.scroll_offset, 4);
    }
}
