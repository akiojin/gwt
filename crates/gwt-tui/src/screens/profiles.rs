//! Profiles management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

/// Current mode of the profiles screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileMode {
    #[default]
    List,
    Create,
    Edit,
    ConfirmDelete,
}

/// A single profile entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileItem {
    pub name: String,
    pub active: bool,
    pub env_count: usize,
    pub description: String,
}

/// State for the profiles screen.
#[derive(Debug, Clone, Default)]
pub struct ProfilesState {
    pub(crate) profiles: Vec<ProfileItem>,
    pub(crate) selected: usize,
    pub(crate) mode: ProfileMode,
    pub(crate) input_name: String,
    pub(crate) input_description: String,
    /// 0 = name field, 1 = description field
    pub(crate) active_field: usize,
}

impl ProfilesState {
    /// Get the currently selected profile, if any.
    pub fn selected_profile(&self) -> Option<&ProfileItem> {
        self.profiles.get(self.selected)
    }

    /// Clamp selected index to list length.
    fn clamp_selected(&mut self) {
        super::clamp_index(&mut self.selected, self.profiles.len());
    }

    /// Clear form input fields.
    fn clear_form(&mut self) {
        self.input_name.clear();
        self.input_description.clear();
        self.active_field = 0;
    }
}

/// Messages specific to the profiles screen.
#[derive(Debug, Clone)]
pub enum ProfilesMessage {
    MoveUp,
    MoveDown,
    ToggleActive,
    StartCreate,
    StartEdit,
    StartDelete,
    Confirm,
    Cancel,
    InputChar(char),
    Backspace,
    NextField,
}

/// Update profiles state in response to a message.
pub fn update(state: &mut ProfilesState, msg: ProfilesMessage) {
    match msg {
        ProfilesMessage::MoveUp => {
            if state.mode == ProfileMode::List {
                super::move_up(&mut state.selected, state.profiles.len());
            }
        }
        ProfilesMessage::MoveDown => {
            if state.mode == ProfileMode::List {
                super::move_down(&mut state.selected, state.profiles.len());
            }
        }
        ProfilesMessage::ToggleActive => {
            if state.mode == ProfileMode::List {
                if let Some(profile) = state.profiles.get_mut(state.selected) {
                    profile.active = !profile.active;
                }
            }
        }
        ProfilesMessage::StartCreate => {
            state.clear_form();
            state.mode = ProfileMode::Create;
        }
        ProfilesMessage::StartEdit => {
            if let Some(profile) = state.profiles.get(state.selected) {
                state.input_name = profile.name.clone();
                state.input_description = profile.description.clone();
                state.active_field = 0;
                state.mode = ProfileMode::Edit;
            }
        }
        ProfilesMessage::StartDelete => {
            if !state.profiles.is_empty() {
                state.mode = ProfileMode::ConfirmDelete;
            }
        }
        ProfilesMessage::Confirm => match state.mode {
            ProfileMode::Create => {
                if !state.input_name.is_empty() {
                    let profile = ProfileItem {
                        name: state.input_name.clone(),
                        active: false,
                        env_count: 0,
                        description: state.input_description.clone(),
                    };
                    state.profiles.push(profile);
                    state.selected = state.profiles.len() - 1;
                }
                state.clear_form();
                state.mode = ProfileMode::List;
            }
            ProfileMode::Edit => {
                if let Some(profile) = state.profiles.get_mut(state.selected) {
                    if !state.input_name.is_empty() {
                        profile.name = state.input_name.clone();
                        profile.description = state.input_description.clone();
                    }
                }
                state.clear_form();
                state.mode = ProfileMode::List;
            }
            ProfileMode::ConfirmDelete => {
                if !state.profiles.is_empty() {
                    state.profiles.remove(state.selected);
                    state.clamp_selected();
                }
                state.mode = ProfileMode::List;
            }
            ProfileMode::List => {}
        },
        ProfilesMessage::Cancel => {
            state.clear_form();
            state.mode = ProfileMode::List;
        }
        ProfilesMessage::InputChar(ch) => match state.mode {
            ProfileMode::Create | ProfileMode::Edit => {
                if state.active_field == 0 {
                    state.input_name.push(ch);
                } else {
                    state.input_description.push(ch);
                }
            }
            _ => {}
        },
        ProfilesMessage::Backspace => match state.mode {
            ProfileMode::Create | ProfileMode::Edit => {
                if state.active_field == 0 {
                    state.input_name.pop();
                } else {
                    state.input_description.pop();
                }
            }
            _ => {}
        },
        ProfilesMessage::NextField => match state.mode {
            ProfileMode::Create | ProfileMode::Edit => {
                state.active_field = (state.active_field + 1) % 2;
            }
            _ => {}
        },
    }
}

/// Render the profiles screen.
pub fn render(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    match state.mode {
        ProfileMode::List => render_list(state, frame, area),
        ProfileMode::Create | ProfileMode::Edit => render_form(state, frame, area),
        ProfileMode::ConfirmDelete => render_confirm_delete(state, frame, area),
    }
}

/// Render the profile list view.
fn render_list(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    if state.profiles.is_empty() {
        let block = Block::default().borders(Borders::ALL).title("Profiles");
        let paragraph = Paragraph::new("No profiles. Press 'c' to create one.")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = state
        .profiles
        .iter()
        .enumerate()
        .map(|(idx, profile)| {
            let active_marker = if profile.active { "[*] " } else { "[ ] " };

            let style = super::list_item_style(idx == state.selected);

            let line = Line::from(vec![
                Span::styled(
                    active_marker.to_string(),
                    if profile.active {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(profile.name.clone(), style),
                Span::styled(
                    format!("  ({} env vars)", profile.env_count),
                    Style::default().fg(Color::Cyan),
                ),
                if !profile.description.is_empty() {
                    Span::styled(
                        format!(" - {}", profile.description),
                        Style::default().fg(Color::DarkGray),
                    )
                } else {
                    Span::raw("")
                },
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title("Profiles");
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Render a single form field with active/inactive styling.
fn render_form_field(title: &str, value: &str, is_active: bool, frame: &mut Frame, area: Rect) {
    let (text_style, border_style) = if is_active {
        (
            Style::default().fg(Color::Yellow),
            Style::default().fg(Color::Yellow),
        )
    } else {
        (Style::default().fg(Color::White), Style::default())
    };

    let display = if is_active {
        format!("{value}_")
    } else {
        value.to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style);
    let paragraph = Paragraph::new(display).block(block).style(text_style);
    frame.render_widget(paragraph, area);
}

/// Render the create/edit form.
fn render_form(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    let title = if state.mode == ProfileMode::Create {
        "Create Profile"
    } else {
        "Edit Profile"
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Name field
            Constraint::Length(3), // Description field
            Constraint::Length(2), // Hints
            Constraint::Min(0),    // Spacer
        ])
        .split(area);

    render_form_field(
        title,
        &format!("Name: {}", state.input_name),
        state.active_field == 0,
        frame,
        chunks[0],
    );
    render_form_field(
        "Description",
        &state.input_description,
        state.active_field == 1,
        frame,
        chunks[1],
    );

    let hints = Paragraph::new(" Tab: next field | Enter: confirm | Esc: cancel")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, chunks[2]);
}

/// Render the delete confirmation dialog.
fn render_confirm_delete(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    let name = state
        .selected_profile()
        .map(|p| p.name.as_str())
        .unwrap_or("unknown");

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Confirm Delete")
        .border_style(Style::default().fg(Color::Red));

    let text = Paragraph::new(format!(
        "Delete profile \"{}\"?\n\nEnter: confirm | Esc: cancel",
        name
    ))
    .block(block)
    .style(Style::default().fg(Color::Red));

    frame.render_widget(text, area);
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn sample_profiles() -> Vec<ProfileItem> {
        vec![
            ProfileItem {
                name: "default".to_string(),
                active: true,
                env_count: 3,
                description: "Default profile".to_string(),
            },
            ProfileItem {
                name: "staging".to_string(),
                active: false,
                env_count: 5,
                description: "Staging env".to_string(),
            },
            ProfileItem {
                name: "production".to_string(),
                active: false,
                env_count: 8,
                description: String::new(),
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = ProfilesState::default();
        assert!(state.profiles.is_empty());
        assert_eq!(state.selected, 0);
        assert_eq!(state.mode, ProfileMode::List);
        assert!(state.input_name.is_empty());
        assert!(state.input_description.is_empty());
        assert_eq!(state.active_field, 0);
    }

    #[test]
    fn move_down_wraps() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.selected, 1);

        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.selected, 2);

        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn move_up_wraps() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        update(&mut state, ProfilesMessage::MoveUp);
        assert_eq!(state.selected, 2); // wraps to last
    }

    #[test]
    fn move_on_empty_is_noop() {
        let mut state = ProfilesState::default();
        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.selected, 0);
        update(&mut state, ProfilesMessage::MoveUp);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn toggle_active_flips() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        assert!(state.profiles[0].active);

        update(&mut state, ProfilesMessage::ToggleActive);
        assert!(!state.profiles[0].active);

        update(&mut state, ProfilesMessage::ToggleActive);
        assert!(state.profiles[0].active);
    }

    #[test]
    fn start_create_sets_mode() {
        let mut state = ProfilesState::default();
        state.input_name = "leftover".to_string();

        update(&mut state, ProfilesMessage::StartCreate);
        assert_eq!(state.mode, ProfileMode::Create);
        assert!(state.input_name.is_empty()); // cleared
    }

    #[test]
    fn start_edit_populates_fields() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 1;

        update(&mut state, ProfilesMessage::StartEdit);
        assert_eq!(state.mode, ProfileMode::Edit);
        assert_eq!(state.input_name, "staging");
        assert_eq!(state.input_description, "Staging env");
    }

    #[test]
    fn start_edit_on_empty_is_noop() {
        let mut state = ProfilesState::default();
        update(&mut state, ProfilesMessage::StartEdit);
        assert_eq!(state.mode, ProfileMode::List);
    }

    #[test]
    fn start_delete_sets_mode() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        update(&mut state, ProfilesMessage::StartDelete);
        assert_eq!(state.mode, ProfileMode::ConfirmDelete);
    }

    #[test]
    fn start_delete_on_empty_is_noop() {
        let mut state = ProfilesState::default();
        update(&mut state, ProfilesMessage::StartDelete);
        assert_eq!(state.mode, ProfileMode::List);
    }

    #[test]
    fn confirm_create_adds_profile() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        state.input_name = "new-profile".to_string();
        state.input_description = "A new one".to_string();

        update(&mut state, ProfilesMessage::Confirm);
        assert_eq!(state.mode, ProfileMode::List);
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].name, "new-profile");
        assert_eq!(state.profiles[0].description, "A new one");
        assert!(!state.profiles[0].active);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn confirm_create_empty_name_does_not_add() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        // input_name is empty

        update(&mut state, ProfilesMessage::Confirm);
        assert_eq!(state.mode, ProfileMode::List);
        assert!(state.profiles.is_empty());
    }

    #[test]
    fn confirm_edit_updates_profile() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 0;
        state.mode = ProfileMode::Edit;
        state.input_name = "renamed".to_string();
        state.input_description = "Updated desc".to_string();

        update(&mut state, ProfilesMessage::Confirm);
        assert_eq!(state.mode, ProfileMode::List);
        assert_eq!(state.profiles[0].name, "renamed");
        assert_eq!(state.profiles[0].description, "Updated desc");
    }

    #[test]
    fn confirm_delete_removes_profile() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 1;
        state.mode = ProfileMode::ConfirmDelete;

        update(&mut state, ProfilesMessage::Confirm);
        assert_eq!(state.mode, ProfileMode::List);
        assert_eq!(state.profiles.len(), 2);
        assert_eq!(state.profiles[0].name, "default");
        assert_eq!(state.profiles[1].name, "production");
        assert_eq!(state.selected, 1); // clamped
    }

    #[test]
    fn cancel_returns_to_list() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        state.input_name = "something".to_string();

        update(&mut state, ProfilesMessage::Cancel);
        assert_eq!(state.mode, ProfileMode::List);
        assert!(state.input_name.is_empty());
    }

    #[test]
    fn input_char_appends_to_active_field() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;

        update(&mut state, ProfilesMessage::InputChar('a'));
        update(&mut state, ProfilesMessage::InputChar('b'));
        assert_eq!(state.input_name, "ab");
        assert!(state.input_description.is_empty());

        state.active_field = 1;
        update(&mut state, ProfilesMessage::InputChar('x'));
        assert_eq!(state.input_description, "x");
    }

    #[test]
    fn backspace_removes_from_active_field() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        state.input_name = "abc".to_string();

        update(&mut state, ProfilesMessage::Backspace);
        assert_eq!(state.input_name, "ab");

        state.active_field = 1;
        state.input_description = "xy".to_string();
        update(&mut state, ProfilesMessage::Backspace);
        assert_eq!(state.input_description, "x");
    }

    #[test]
    fn backspace_on_empty_is_noop() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        update(&mut state, ProfilesMessage::Backspace);
        assert!(state.input_name.is_empty());
    }

    #[test]
    fn next_field_cycles() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        assert_eq!(state.active_field, 0);

        update(&mut state, ProfilesMessage::NextField);
        assert_eq!(state.active_field, 1);

        update(&mut state, ProfilesMessage::NextField);
        assert_eq!(state.active_field, 0);
    }

    #[test]
    fn next_field_noop_in_list_mode() {
        let mut state = ProfilesState::default();
        update(&mut state, ProfilesMessage::NextField);
        assert_eq!(state.active_field, 0);
    }

    #[test]
    fn render_list_does_not_panic() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
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
        assert!(text.contains("Profiles"));
    }

    #[test]
    fn render_empty_list_does_not_panic() {
        let state = ProfilesState::default();
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
    fn render_form_does_not_panic() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::Create;
        state.input_name = "test".to_string();
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
    fn render_confirm_delete_does_not_panic() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.mode = ProfileMode::ConfirmDelete;
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
    fn input_ignored_in_list_mode() {
        let mut state = ProfilesState::default();
        update(&mut state, ProfilesMessage::InputChar('z'));
        assert!(state.input_name.is_empty());
    }

    #[test]
    fn delete_last_item_clamps() {
        let mut state = ProfilesState::default();
        state.profiles = vec![ProfileItem {
            name: "only".to_string(),
            active: false,
            env_count: 0,
            description: String::new(),
        }];
        state.selected = 0;
        state.mode = ProfileMode::ConfirmDelete;

        update(&mut state, ProfilesMessage::Confirm);
        assert!(state.profiles.is_empty());
        assert_eq!(state.selected, 0);
    }
}
