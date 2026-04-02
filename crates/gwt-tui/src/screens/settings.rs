//! Settings management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};

/// Settings category tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsCategory {
    #[default]
    General,
    Worktree,
    Agent,
    CustomAgents,
    Environment,
    Ai,
    Voice,
}

impl SettingsCategory {
    /// All categories in display order.
    pub const ALL: [SettingsCategory; 7] = [
        SettingsCategory::General,
        SettingsCategory::Worktree,
        SettingsCategory::Agent,
        SettingsCategory::CustomAgents,
        SettingsCategory::Environment,
        SettingsCategory::Ai,
        SettingsCategory::Voice,
    ];

    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Worktree => "Worktree",
            Self::Agent => "Agent",
            Self::CustomAgents => "Custom Agents",
            Self::Environment => "Environment",
            Self::Ai => "AI",
            Self::Voice => "Voice",
        }
    }

    /// Cycle to next category.
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Cycle to previous category.
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|c| *c == self).unwrap_or(0);
        if idx == 0 {
            Self::ALL[Self::ALL.len() - 1]
        } else {
            Self::ALL[idx - 1]
        }
    }
}

/// Field type for a setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Bool,
    Path,
}

/// A single setting field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingField {
    pub label: String,
    pub value: String,
    pub field_type: FieldType,
}

/// State for the settings screen.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub(crate) category: SettingsCategory,
    pub(crate) fields: Vec<SettingField>,
    pub(crate) selected: usize,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
    /// Last save error, if any.
    pub(crate) save_error: Option<String>,
}

impl SettingsState {
    /// Get the currently selected field, if any.
    pub fn selected_field(&self) -> Option<&SettingField> {
        self.fields.get(self.selected)
    }

    /// Load fields for the current category.
    pub fn load_category_fields(&mut self) {
        self.fields = fields_for_category(self.category);
        self.selected = 0;
        self.editing = false;
        self.edit_buffer.clear();
    }
}

/// Return default fields for a given category.
pub fn fields_for_category(category: SettingsCategory) -> Vec<SettingField> {
    match category {
        SettingsCategory::General => vec![
            SettingField {
                label: "Theme".to_string(),
                value: "dark".to_string(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "Language".to_string(),
                value: "en".to_string(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "Auto-save".to_string(),
                value: "true".to_string(),
                field_type: FieldType::Bool,
            },
            SettingField {
                label: "Log level".to_string(),
                value: "info".to_string(),
                field_type: FieldType::Text,
            },
        ],
        SettingsCategory::Worktree => vec![
            SettingField {
                label: "Default path".to_string(),
                value: "~/.gwt/worktrees".to_string(),
                field_type: FieldType::Path,
            },
            SettingField {
                label: "Auto-clean".to_string(),
                value: "false".to_string(),
                field_type: FieldType::Bool,
            },
            SettingField {
                label: "Max worktrees".to_string(),
                value: "10".to_string(),
                field_type: FieldType::Text,
            },
        ],
        SettingsCategory::Agent => vec![
            SettingField {
                label: "Default agent".to_string(),
                value: "claude".to_string(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "Auto-start".to_string(),
                value: "false".to_string(),
                field_type: FieldType::Bool,
            },
            SettingField {
                label: "Timeout (s)".to_string(),
                value: "300".to_string(),
                field_type: FieldType::Text,
            },
        ],
        SettingsCategory::CustomAgents => vec![
            SettingField {
                label: "Config path".to_string(),
                value: "~/.gwt/agents".to_string(),
                field_type: FieldType::Path,
            },
            SettingField {
                label: "Enable custom".to_string(),
                value: "true".to_string(),
                field_type: FieldType::Bool,
            },
        ],
        SettingsCategory::Environment => vec![
            SettingField {
                label: "Shell".to_string(),
                value: "/bin/zsh".to_string(),
                field_type: FieldType::Path,
            },
            SettingField {
                label: "PATH prefix".to_string(),
                value: String::new(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "Inherit env".to_string(),
                value: "true".to_string(),
                field_type: FieldType::Bool,
            },
        ],
        SettingsCategory::Ai => vec![
            SettingField {
                label: "Provider".to_string(),
                value: "anthropic".to_string(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "Model".to_string(),
                value: "claude-sonnet-4-20250514".to_string(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "API key set".to_string(),
                value: "true".to_string(),
                field_type: FieldType::Bool,
            },
        ],
        SettingsCategory::Voice => vec![
            SettingField {
                label: "Enabled".to_string(),
                value: "false".to_string(),
                field_type: FieldType::Bool,
            },
            SettingField {
                label: "Input device".to_string(),
                value: "default".to_string(),
                field_type: FieldType::Text,
            },
            SettingField {
                label: "Language".to_string(),
                value: "en-US".to_string(),
                field_type: FieldType::Text,
            },
        ],
    }
}

/// Messages specific to the settings screen.
#[derive(Debug, Clone)]
pub enum SettingsMessage {
    MoveUp,
    MoveDown,
    NextCategory,
    PrevCategory,
    StartEdit,
    EndEdit,
    CancelEdit,
    InputChar(char),
    Backspace,
    ToggleBool,
    Save,
}

/// Toggle a bool field's value between "true" and "false".
fn toggle_bool_field(field: &mut SettingField) {
    if field.field_type == FieldType::Bool {
        field.value = if field.value == "true" {
            "false".to_string()
        } else {
            "true".to_string()
        };
    }
}

/// Persist current settings fields to gwt-config's global config.
///
/// Reads the current global Settings, applies matching fields from the TUI state,
/// and writes back. Returns an error string on failure.
fn save_settings_to_config(state: &SettingsState) -> Result<(), String> {
    use gwt_config::Settings;

    Settings::update_global(|settings| {
        for field in &state.fields {
            match (state.category, field.label.as_str()) {
                // General
                (SettingsCategory::General, "Log level") => {
                    settings.debug = field.value == "debug";
                }
                // Worktree
                (SettingsCategory::Worktree, "Default path") => {
                    if field.value.is_empty() || field.value == "~/.gwt/worktrees" {
                        settings.worktree_root = None;
                    } else {
                        settings.worktree_root =
                            Some(std::path::PathBuf::from(&field.value));
                    }
                }
                // Agent
                (SettingsCategory::Agent, "Default agent") => {
                    if field.value.is_empty() {
                        settings.agent.default_agent = None;
                    } else {
                        settings.agent.default_agent = Some(field.value.clone());
                    }
                }
                // Voice
                (SettingsCategory::Voice, "Enabled") => {
                    settings.voice.enabled = field.value == "true";
                }
                (SettingsCategory::Voice, "Language") => {
                    if field.value.is_empty() {
                        settings.voice.language = None;
                    } else {
                        settings.voice.language = Some(field.value.clone());
                    }
                }
                _ => {} // Other fields have no backend mapping yet
            }
        }
        Ok(())
    })
    .map_err(|e| format!("{e}"))
}

/// Update settings state in response to a message.
pub fn update(state: &mut SettingsState, msg: SettingsMessage) {
    match msg {
        SettingsMessage::MoveUp => {
            if !state.editing {
                super::move_up(&mut state.selected, state.fields.len());
            }
        }
        SettingsMessage::MoveDown => {
            if !state.editing {
                super::move_down(&mut state.selected, state.fields.len());
            }
        }
        SettingsMessage::NextCategory => {
            if !state.editing {
                state.category = state.category.next();
                state.load_category_fields();
            }
        }
        SettingsMessage::PrevCategory => {
            if !state.editing {
                state.category = state.category.prev();
                state.load_category_fields();
            }
        }
        SettingsMessage::StartEdit => {
            if !state.editing {
                if let Some(field) = state.fields.get(state.selected) {
                    if field.field_type == FieldType::Bool {
                        toggle_bool_field(&mut state.fields[state.selected]);
                    } else {
                        state.edit_buffer = field.value.clone();
                        state.editing = true;
                    }
                }
            }
        }
        SettingsMessage::EndEdit => {
            if state.editing {
                if let Some(field) = state.fields.get_mut(state.selected) {
                    field.value = state.edit_buffer.clone();
                }
                state.editing = false;
                state.edit_buffer.clear();
            }
        }
        SettingsMessage::CancelEdit => {
            state.editing = false;
            state.edit_buffer.clear();
        }
        SettingsMessage::InputChar(ch) => {
            if state.editing {
                state.edit_buffer.push(ch);
            }
        }
        SettingsMessage::Backspace => {
            if state.editing {
                state.edit_buffer.pop();
            }
        }
        SettingsMessage::ToggleBool => {
            if !state.editing {
                if let Some(field) = state.fields.get_mut(state.selected) {
                    toggle_bool_field(field);
                }
            }
        }
        SettingsMessage::Save => {
            if let Err(e) = save_settings_to_config(state) {
                state.save_error = Some(e);
            } else {
                state.save_error = None;
            }
        }
    }
}

/// Render the settings screen.
pub fn render(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Category tabs
            Constraint::Min(0),   // Fields
        ])
        .split(area);

    render_category_tabs(state, frame, chunks[0]);
    render_fields(state, frame, chunks[1]);
}

/// Render the category tab bar.
fn render_category_tabs(state: &SettingsState, frame: &mut Frame, area: Rect) {
    let titles: Vec<Line> = SettingsCategory::ALL
        .iter()
        .map(|c| Line::from(c.label()))
        .collect();

    let active_idx = SettingsCategory::ALL
        .iter()
        .position(|c| *c == state.category)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Settings"))
        .select(active_idx)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

/// Render the fields list for the current category.
fn render_fields(state: &SettingsState, frame: &mut Frame, area: Rect) {
    if state.fields.is_empty() {
        let block = Block::default().borders(Borders::ALL);
        let paragraph = Paragraph::new("No settings in this category")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .fields
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let is_selected = idx == state.selected;
            let is_editing = is_selected && state.editing;

            let label_style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let value_display = if is_editing {
                format!("{}_", state.edit_buffer)
            } else {
                field.value.clone()
            };

            let value_style = match (&field.field_type, is_editing) {
                (_, true) => Style::default().fg(Color::Yellow),
                (FieldType::Bool, false) => {
                    if field.value == "true" {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::Red)
                    }
                }
                (FieldType::Path, false) => Style::default().fg(Color::Cyan),
                (FieldType::Text, false) => Style::default().fg(Color::White),
            };

            let type_indicator = match field.field_type {
                FieldType::Text => "T",
                FieldType::Bool => "B",
                FieldType::Path => "P",
            };

            let bg_style = if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("[{}] ", type_indicator),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("{}: ", field.label), label_style),
                Span::styled(value_display, value_style),
            ]);
            ListItem::new(line).style(bg_style)
        })
        .collect();

    let hints = if state.editing {
        " Enter: save | Esc: cancel"
    } else {
        " Enter: edit | Space: toggle bool | Tab/Shift+Tab: category"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("{}{}", state.category.label(), hints));
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn state_with_fields() -> SettingsState {
        let mut state = SettingsState::default();
        state.load_category_fields();
        state
    }

    #[test]
    fn default_state() {
        let state = SettingsState::default();
        assert_eq!(state.category, SettingsCategory::General);
        assert!(state.fields.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.editing);
        assert!(state.edit_buffer.is_empty());
    }

    #[test]
    fn load_category_fields_populates() {
        let state = state_with_fields();
        assert!(!state.fields.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.editing);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = state_with_fields();
        let len = state.fields.len();

        for _ in 0..len {
            update(&mut state, SettingsMessage::MoveDown);
        }
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = state_with_fields();

        update(&mut state, SettingsMessage::MoveUp);
        assert_eq!(state.selected, state.fields.len() - 1); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = SettingsState::default();
        update(&mut state, SettingsMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, SettingsMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_blocked_during_editing() {
        let mut state = state_with_fields();
        state.editing = true;

        update(&mut state, SettingsMessage::MoveDown);
        assert_eq!(state.selected, 0); // did not move
    }

    #[test]
    fn next_category_cycles() {
        let mut state = state_with_fields();
        assert_eq!(state.category, SettingsCategory::General);

        update(&mut state, SettingsMessage::NextCategory);
        assert_eq!(state.category, SettingsCategory::Worktree);
        assert!(!state.fields.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn prev_category_cycles() {
        let mut state = state_with_fields();
        assert_eq!(state.category, SettingsCategory::General);

        update(&mut state, SettingsMessage::PrevCategory);
        assert_eq!(state.category, SettingsCategory::Voice);
    }

    #[test]
    fn category_change_blocked_during_editing() {
        let mut state = state_with_fields();
        state.editing = true;

        update(&mut state, SettingsMessage::NextCategory);
        assert_eq!(state.category, SettingsCategory::General);
    }

    #[test]
    fn start_edit_text_field() {
        let mut state = state_with_fields();
        // First field is "Theme" (Text)
        assert_eq!(state.fields[0].field_type, FieldType::Text);

        update(&mut state, SettingsMessage::StartEdit);
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "dark");
    }

    #[test]
    fn start_edit_bool_toggles_instead() {
        let mut state = state_with_fields();
        // Find a Bool field — "Auto-save" at index 2
        state.selected = 2;
        assert_eq!(state.fields[2].field_type, FieldType::Bool);
        assert_eq!(state.fields[2].value, "true");

        update(&mut state, SettingsMessage::StartEdit);
        assert!(!state.editing); // did not enter edit mode
        assert_eq!(state.fields[2].value, "false"); // toggled
    }

    #[test]
    fn end_edit_saves_buffer() {
        let mut state = state_with_fields();
        state.editing = true;
        state.edit_buffer = "light".to_string();

        update(&mut state, SettingsMessage::EndEdit);
        assert!(!state.editing);
        assert_eq!(state.fields[0].value, "light");
        assert!(state.edit_buffer.is_empty());
    }

    #[test]
    fn cancel_edit_discards() {
        let mut state = state_with_fields();
        let original = state.fields[0].value.clone();
        state.editing = true;
        state.edit_buffer = "something-else".to_string();

        update(&mut state, SettingsMessage::CancelEdit);
        assert!(!state.editing);
        assert_eq!(state.fields[0].value, original); // unchanged
        assert!(state.edit_buffer.is_empty());
    }

    #[test]
    fn input_char_appends_in_edit_mode() {
        let mut state = state_with_fields();
        state.editing = true;
        state.edit_buffer.clear();

        update(&mut state, SettingsMessage::InputChar('a'));
        update(&mut state, SettingsMessage::InputChar('b'));
        assert_eq!(state.edit_buffer, "ab");
    }

    #[test]
    fn input_char_ignored_outside_edit() {
        let mut state = state_with_fields();
        update(&mut state, SettingsMessage::InputChar('x'));
        assert!(state.edit_buffer.is_empty());
    }

    #[test]
    fn backspace_removes_in_edit_mode() {
        let mut state = state_with_fields();
        state.editing = true;
        state.edit_buffer = "abc".to_string();

        update(&mut state, SettingsMessage::Backspace);
        assert_eq!(state.edit_buffer, "ab");
    }

    #[test]
    fn backspace_on_empty_is_noop() {
        let mut state = state_with_fields();
        state.editing = true;
        state.edit_buffer.clear();

        update(&mut state, SettingsMessage::Backspace);
        assert!(state.edit_buffer.is_empty());
    }

    #[test]
    fn toggle_bool_flips_value() {
        let mut state = state_with_fields();
        state.selected = 2; // Auto-save (Bool)
        assert_eq!(state.fields[2].value, "true");

        update(&mut state, SettingsMessage::ToggleBool);
        assert_eq!(state.fields[2].value, "false");

        update(&mut state, SettingsMessage::ToggleBool);
        assert_eq!(state.fields[2].value, "true");
    }

    #[test]
    fn toggle_bool_noop_on_text_field() {
        let mut state = state_with_fields();
        state.selected = 0; // Theme (Text)
        let original = state.fields[0].value.clone();

        update(&mut state, SettingsMessage::ToggleBool);
        assert_eq!(state.fields[0].value, original);
    }

    #[test]
    fn toggle_bool_noop_during_editing() {
        let mut state = state_with_fields();
        state.selected = 2;
        state.editing = true;
        let original = state.fields[2].value.clone();

        update(&mut state, SettingsMessage::ToggleBool);
        assert_eq!(state.fields[2].value, original);
    }

    #[test]
    fn fields_for_all_categories_non_empty() {
        for cat in SettingsCategory::ALL {
            let fields = fields_for_category(cat);
            assert!(
                !fields.is_empty(),
                "Category {:?} has no fields",
                cat
            );
        }
    }

    #[test]
    fn category_cycle_full_round() {
        let mut cat = SettingsCategory::General;
        for _ in 0..7 {
            cat = cat.next();
        }
        assert_eq!(cat, SettingsCategory::General); // full cycle
    }

    #[test]
    fn category_prev_full_round() {
        let mut cat = SettingsCategory::General;
        for _ in 0..7 {
            cat = cat.prev();
        }
        assert_eq!(cat, SettingsCategory::General); // full cycle
    }

    #[test]
    fn render_with_fields_does_not_panic() {
        let state = state_with_fields();
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
        assert!(text.contains("Settings"));
    }

    #[test]
    fn render_empty_does_not_panic() {
        let state = SettingsState::default();
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
    fn render_editing_does_not_panic() {
        let mut state = state_with_fields();
        state.editing = true;
        state.edit_buffer = "new-value".to_string();
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
    fn selected_field_returns_correct() {
        let mut state = state_with_fields();
        state.selected = 1;
        let field = state.selected_field().unwrap();
        assert_eq!(field.label, "Language");
    }

    #[test]
    fn selected_field_none_when_empty() {
        let state = SettingsState::default();
        assert!(state.selected_field().is_none());
    }
}
