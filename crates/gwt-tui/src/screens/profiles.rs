//! Profiles screen — environment profiles management (dedicated tab)
//!
//! Extracted from settings.rs into a standalone screen with its own
//! state, messages, key handling, update, and render.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::compat::config::{Profile, ProfilesConfig};

// ---------------------------------------------------------------------------
// Profile mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProfileMode {
    #[default]
    List,
    Create,
    Edit(String),
    Delete(String),
}

// ---------------------------------------------------------------------------
// Profiles state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ProfilesState {
    pub profiles_config: Option<ProfilesConfig>,
    pub selected: usize,
    pub mode: ProfileMode,
    pub form_name: String,
    pub form_description: String,
    pub form_cursor: usize,
    pub form_field: FormField,
    pub delete_confirm: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FormField {
    #[default]
    Name,
    Description,
}

impl Default for ProfilesState {
    fn default() -> Self {
        Self {
            profiles_config: None,
            selected: 0,
            mode: ProfileMode::List,
            form_name: String::new(),
            form_description: String::new(),
            form_cursor: 0,
            form_field: FormField::Name,
            delete_confirm: false,
            error_message: None,
        }
    }
}

impl ProfilesState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&mut self) {
        match ProfilesConfig::load() {
            Ok(config) => {
                self.profiles_config = Some(config);
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to load profiles: {e}"));
            }
        }
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles_config
            .as_ref()
            .map(|c| {
                let mut names: Vec<String> = c.profiles.keys().cloned().collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    }

    pub fn profile_count(&self) -> usize {
        self.profiles_config
            .as_ref()
            .map(|c| c.profiles.len())
            .unwrap_or(0)
    }

    pub fn selected_name(&self) -> Option<String> {
        let names = self.profile_names();
        names.get(self.selected).cloned()
    }

    pub fn selected_profile(&self) -> Option<&Profile> {
        let name = self.selected_name()?;
        self.profiles_config.as_ref()?.profiles.get(&name)
    }

    pub fn is_active(&self, name: &str) -> bool {
        self.profiles_config
            .as_ref()
            .and_then(|c| c.active.as_deref())
            .is_some_and(|active| active == name)
    }

    pub fn clamp_selection(&mut self) {
        let count = self.profile_count();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ProfilesMessage {
    Refresh,
    SelectNext,
    SelectPrev,
    ToggleActive,
    EnterCreate,
    EnterEdit,
    EnterDelete,
    FormInput(char),
    FormBackspace,
    FormNextField,
    FormPrevField,
    FormSubmit,
    FormCancel,
    DeleteConfirm,
    DeleteCancel,
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

pub fn handle_key(state: &ProfilesState, key: &KeyEvent) -> Option<ProfilesMessage> {
    match &state.mode {
        ProfileMode::List => handle_list_key(key),
        ProfileMode::Create | ProfileMode::Edit(_) => handle_form_key(key),
        ProfileMode::Delete(_) => handle_delete_key(key),
    }
}

fn handle_list_key(key: &KeyEvent) -> Option<ProfilesMessage> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(ProfilesMessage::SelectNext),
        KeyCode::Char('k') | KeyCode::Up => Some(ProfilesMessage::SelectPrev),
        KeyCode::Enter | KeyCode::Char(' ') => Some(ProfilesMessage::ToggleActive),
        KeyCode::Char('n') | KeyCode::Char('a') => Some(ProfilesMessage::EnterCreate),
        KeyCode::Char('e') => Some(ProfilesMessage::EnterEdit),
        KeyCode::Char('d') => Some(ProfilesMessage::EnterDelete),
        KeyCode::Char('r') => Some(ProfilesMessage::Refresh),
        _ => None,
    }
}

fn handle_form_key(key: &KeyEvent) -> Option<ProfilesMessage> {
    match key.code {
        KeyCode::Esc => Some(ProfilesMessage::FormCancel),
        KeyCode::Enter => Some(ProfilesMessage::FormSubmit),
        KeyCode::Tab => Some(ProfilesMessage::FormNextField),
        KeyCode::BackTab => Some(ProfilesMessage::FormPrevField),
        KeyCode::Backspace => Some(ProfilesMessage::FormBackspace),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(ProfilesMessage::FormCancel)
        }
        KeyCode::Char(c) => Some(ProfilesMessage::FormInput(c)),
        _ => None,
    }
}

fn handle_delete_key(key: &KeyEvent) -> Option<ProfilesMessage> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => Some(ProfilesMessage::DeleteConfirm),
        KeyCode::Char('n') | KeyCode::Esc => Some(ProfilesMessage::DeleteCancel),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

pub fn update(state: &mut ProfilesState, msg: ProfilesMessage) {
    match msg {
        ProfilesMessage::Refresh => state.load(),
        ProfilesMessage::SelectNext => {
            let count = state.profile_count();
            if count > 0 && state.selected < count - 1 {
                state.selected += 1;
            }
        }
        ProfilesMessage::SelectPrev => {
            state.selected = state.selected.saturating_sub(1);
        }
        ProfilesMessage::ToggleActive => {
            if let Some(name) = state.selected_name() {
                let is_active = state.is_active(&name);
                if let Some(ref mut config) = state.profiles_config {
                    if is_active {
                        config.active = None;
                    } else {
                        let _ = config.set_active(&name);
                    }
                }
            }
        }
        ProfilesMessage::EnterCreate => {
            state.form_name.clear();
            state.form_description.clear();
            state.form_cursor = 0;
            state.form_field = FormField::Name;
            state.mode = ProfileMode::Create;
        }
        ProfilesMessage::EnterEdit => {
            if let Some(profile) = state.selected_profile().cloned() {
                state.form_name = profile.name.clone();
                state.form_description = profile.description.clone();
                state.form_cursor = profile.name.len();
                state.form_field = FormField::Name;
                state.mode = ProfileMode::Edit(profile.name);
            }
        }
        ProfilesMessage::EnterDelete => {
            if let Some(name) = state.selected_name() {
                state.delete_confirm = false;
                state.mode = ProfileMode::Delete(name);
            }
        }
        ProfilesMessage::FormInput(c) => {
            let cursor = state.form_cursor;
            let field = match state.form_field {
                FormField::Name => &mut state.form_name,
                FormField::Description => &mut state.form_description,
            };
            field.insert(cursor, c);
            state.form_cursor += 1;
        }
        ProfilesMessage::FormBackspace => {
            if state.form_cursor > 0 {
                state.form_cursor -= 1;
                let cursor = state.form_cursor;
                let field = match state.form_field {
                    FormField::Name => &mut state.form_name,
                    FormField::Description => &mut state.form_description,
                };
                field.remove(cursor);
            }
        }
        ProfilesMessage::FormNextField => {
            state.form_field = match state.form_field {
                FormField::Name => FormField::Description,
                FormField::Description => FormField::Name,
            };
            state.form_cursor = match state.form_field {
                FormField::Name => state.form_name.len(),
                FormField::Description => state.form_description.len(),
            };
        }
        ProfilesMessage::FormPrevField => {
            update(state, ProfilesMessage::FormNextField);
        }
        ProfilesMessage::FormSubmit => {
            if state.form_name.trim().is_empty() {
                state.error_message = Some("Name is required".to_string());
                return;
            }
            let name = state.form_name.trim().to_string();
            let profile = Profile {
                name: name.clone(),
                description: state.form_description.clone(),
                env: HashMap::new(),
                disabled_env: Vec::new(),
                ai: None,
                ai_enabled: None,
            };

            match &state.mode {
                ProfileMode::Create => {
                    if let Some(ref mut config) = state.profiles_config {
                        if config.profiles.contains_key(&name) {
                            state.error_message =
                                Some("Profile already exists".to_string());
                            return;
                        }
                        config.profiles.insert(name, profile);
                    }
                }
                ProfileMode::Edit(old_name) => {
                    if let Some(ref mut config) = state.profiles_config {
                        if old_name != &name {
                            config.profiles.remove(old_name);
                        }
                        config.profiles.insert(name, profile);
                    }
                }
                _ => {}
            }
            state.mode = ProfileMode::List;
            state.error_message = None;
            state.clamp_selection();
        }
        ProfilesMessage::FormCancel => {
            state.mode = ProfileMode::List;
            state.error_message = None;
        }
        ProfilesMessage::DeleteConfirm => {
            if let ProfileMode::Delete(ref name) = state.mode {
                let name = name.clone();
                if let Some(ref mut config) = state.profiles_config {
                    config.profiles.remove(&name);
                    if config.active.as_deref() == Some(&name) {
                        config.active = None;
                    }
                }
            }
            state.mode = ProfileMode::List;
            state.clamp_selection();
        }
        ProfilesMessage::DeleteCancel => {
            state.mode = ProfileMode::List;
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render(state: &ProfilesState, buf: &mut Buffer, area: Rect) {
    if area.height < 3 || area.width < 10 {
        return;
    }

    match &state.mode {
        ProfileMode::List => render_list(state, buf, area),
        ProfileMode::Create | ProfileMode::Edit(_) => render_form(state, buf, area),
        ProfileMode::Delete(name) => render_delete_confirm(name, buf, area),
    }
}

fn render_list(state: &ProfilesState, buf: &mut Buffer, area: Rect) {
    let header_height = 2u16;
    let list_height = area.height.saturating_sub(header_height);
    let header_area = Rect::new(area.x, area.y, area.width, header_height);
    let list_area = Rect::new(area.x, area.y + header_height, area.width, list_height);

    // Header
    let count = state.profile_count();
    let title = format!(" Profiles ({count})");
    let title_line = Line::from(Span::styled(title, Style::default().fg(Color::White).bold()));
    buf.set_line(header_area.x, header_area.y, &title_line, header_area.width);

    let hint_line = Line::from(vec![
        Span::styled("[n] New", Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[e] Edit", Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[d] Delete", Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled("[Enter] Toggle Active", Style::default().fg(Color::DarkGray)),
    ]);
    buf.set_line(
        header_area.x,
        header_area.y + 1,
        &hint_line,
        header_area.width,
    );

    // Profile list
    let names = state.profile_names();
    if names.is_empty() {
        let msg = Paragraph::new("No profiles. Press 'n' to create one.")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        let y = list_area.y + list_area.height / 2;
        Widget::render(msg, Rect::new(list_area.x, y, list_area.width, 1), buf);
        return;
    }

    for (i, name) in names.iter().enumerate() {
        if i as u16 >= list_area.height {
            break;
        }
        let y = list_area.y + i as u16;
        let is_selected = i == state.selected;
        let is_active = state.is_active(name);

        let profile = state
            .profiles_config
            .as_ref()
            .and_then(|c| c.profiles.get(name));
        let env_count = profile.map(|p| p.env.len()).unwrap_or(0);
        let desc = profile
            .map(|p| {
                if p.description.is_empty() {
                    String::new()
                } else {
                    format!(" - {}", p.description)
                }
            })
            .unwrap_or_default();

        let mut spans = Vec::new();

        // Selection indicator
        let sel_char = if is_selected { ">" } else { " " };
        spans.push(Span::styled(
            sel_char,
            if is_selected {
                Style::default().fg(Color::White).bold()
            } else {
                Style::default().fg(Color::DarkGray)
            },
        ));

        // Active indicator
        let active_marker = if is_active { "*" } else { " " };
        spans.push(Span::styled(
            active_marker,
            Style::default().fg(Color::Green),
        ));

        // Profile name
        spans.push(Span::styled(
            format!(" {name}"),
            if is_active {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            },
        ));

        // Env var count
        if env_count > 0 {
            spans.push(Span::styled(
                format!(" ({env_count} vars)"),
                Style::default().fg(Color::Cyan),
            ));
        }

        // Description
        if !desc.is_empty() {
            spans.push(Span::styled(desc, Style::default().fg(Color::DarkGray)));
        }

        if is_selected {
            for col in area.x..area.x + area.width {
                buf[(col, y)].set_style(Style::default().bg(Color::Rgb(40, 40, 60)));
            }
        }

        let line = Line::from(spans);
        buf.set_line(area.x, y, &line, area.width);
    }
}

fn render_form(state: &ProfilesState, buf: &mut Buffer, area: Rect) {
    let is_edit = matches!(state.mode, ProfileMode::Edit(_));
    let title = if is_edit { " Edit Profile " } else { " Create Profile " };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title);
    let inner = block.inner(area);
    Widget::render(block, area, buf);

    if inner.height < 6 || inner.width < 20 {
        return;
    }

    let fields = [("Name", &state.form_name), ("Description", &state.form_description)];

    for (i, (label, value)) in fields.iter().enumerate() {
        let y = inner.y + (i as u16) * 3;
        if y + 2 > inner.y + inner.height {
            break;
        }

        let is_active = match (i, state.form_field) {
            (0, FormField::Name) | (1, FormField::Description) => true,
            _ => false,
        };

        let field_style = if is_active {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let label_line = Line::from(Span::styled(format!(" {label}:"), field_style));
        buf.set_line(inner.x, y, &label_line, inner.width);

        let display = if is_active {
            let mut text = (*value).clone();
            let cursor_pos = state.form_cursor.min(text.len());
            text.insert(cursor_pos, '|');
            text
        } else {
            (*value).clone()
        };
        let value_line = Line::from(Span::styled(format!(" {display}"), Style::default().fg(Color::White)));
        buf.set_line(inner.x, y + 1, &value_line, inner.width);
    }

    // Error message
    if let Some(ref err) = state.error_message {
        let err_y = inner.y + inner.height - 1;
        let err_line = Line::from(Span::styled(err.as_str(), Style::default().fg(Color::Red)));
        buf.set_line(inner.x + 1, err_y, &err_line, inner.width - 2);
    }
}

fn render_delete_confirm(name: &str, buf: &mut Buffer, area: Rect) {
    let text = vec![
        Line::from(format!("Delete profile \"{name}\"?")),
        Line::from(""),
        Line::from("Press 'y' to confirm, 'n' to cancel."),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Confirm Delete ");
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block);
    Widget::render(paragraph, area, buf);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn test_config() -> ProfilesConfig {
        let mut profiles = HashMap::new();
        profiles.insert(
            "default".to_string(),
            Profile {
                name: "default".to_string(),
                description: "Default profile".to_string(),
                env: HashMap::new(),
                disabled_env: Vec::new(),
                ai: None,
                ai_enabled: None,
            },
        );
        profiles.insert(
            "dev".to_string(),
            Profile {
                name: "dev".to_string(),
                description: "Development".to_string(),
                env: {
                    let mut m = HashMap::new();
                    m.insert("DEBUG".to_string(), "1".to_string());
                    m
                },
                disabled_env: Vec::new(),
                ai: None,
                ai_enabled: None,
            },
        );
        ProfilesConfig {
            profiles,
            active: Some("default".to_string()),
            version: None,
        }
    }

    fn test_state() -> ProfilesState {
        let mut state = ProfilesState::new();
        state.profiles_config = Some(test_config());
        state
    }

    // -- State tests --

    #[test]
    fn profile_names_sorted() {
        let state = test_state();
        let names = state.profile_names();
        assert_eq!(names, vec!["default", "dev"]);
    }

    #[test]
    fn profile_count() {
        let state = test_state();
        assert_eq!(state.profile_count(), 2);
    }

    #[test]
    fn selected_name() {
        let state = test_state();
        assert_eq!(state.selected_name(), Some("default".to_string()));
    }

    #[test]
    fn is_active() {
        let state = test_state();
        assert!(state.is_active("default"));
        assert!(!state.is_active("dev"));
    }

    #[test]
    fn clamp_selection() {
        let mut state = test_state();
        state.selected = 99;
        state.clamp_selection();
        assert_eq!(state.selected, 1);
    }

    // -- Key handling tests --

    #[test]
    fn list_key_navigation() {
        let state = test_state();
        let key_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_j),
            Some(ProfilesMessage::SelectNext)
        ));

        let key_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_k),
            Some(ProfilesMessage::SelectPrev)
        ));
    }

    #[test]
    fn list_key_actions() {
        let state = test_state();
        let key_n = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_n),
            Some(ProfilesMessage::EnterCreate)
        ));

        let key_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_e),
            Some(ProfilesMessage::EnterEdit)
        ));

        let key_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_d),
            Some(ProfilesMessage::EnterDelete)
        ));
    }

    #[test]
    fn form_key_input() {
        let mut state = test_state();
        state.mode = ProfileMode::Create;

        let key_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_a),
            Some(ProfilesMessage::FormInput('a'))
        ));
    }

    #[test]
    fn delete_key_confirm() {
        let mut state = test_state();
        state.mode = ProfileMode::Delete("dev".to_string());

        let key_y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert!(matches!(
            handle_key(&state, &key_y),
            Some(ProfilesMessage::DeleteConfirm)
        ));
    }

    // -- Update tests --

    #[test]
    fn update_select_next_prev() {
        let mut state = test_state();
        assert_eq!(state.selected, 0);
        update(&mut state, ProfilesMessage::SelectNext);
        assert_eq!(state.selected, 1);
        update(&mut state, ProfilesMessage::SelectPrev);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn update_toggle_active() {
        let mut state = test_state();
        // default is active, toggle it off
        update(&mut state, ProfilesMessage::ToggleActive);
        assert!(!state.is_active("default"));

        // toggle it back on
        update(&mut state, ProfilesMessage::ToggleActive);
        assert!(state.is_active("default"));
    }

    #[test]
    fn update_create_profile() {
        let mut state = test_state();
        update(&mut state, ProfilesMessage::EnterCreate);
        assert!(matches!(state.mode, ProfileMode::Create));

        // Type name
        update(&mut state, ProfilesMessage::FormInput('p'));
        update(&mut state, ProfilesMessage::FormInput('r'));
        update(&mut state, ProfilesMessage::FormInput('o'));
        update(&mut state, ProfilesMessage::FormInput('d'));
        assert_eq!(state.form_name, "prod");

        update(&mut state, ProfilesMessage::FormSubmit);
        assert!(matches!(state.mode, ProfileMode::List));
        assert_eq!(state.profile_count(), 3);
        assert!(state
            .profiles_config
            .as_ref()
            .unwrap()
            .profiles
            .contains_key("prod"));
    }

    #[test]
    fn update_create_duplicate_fails() {
        let mut state = test_state();
        update(&mut state, ProfilesMessage::EnterCreate);
        state.form_name = "default".to_string();
        update(&mut state, ProfilesMessage::FormSubmit);
        assert!(state.error_message.is_some());
        assert!(matches!(state.mode, ProfileMode::Create));
    }

    #[test]
    fn update_edit_profile() {
        let mut state = test_state();
        state.selected = 1; // "dev"
        update(&mut state, ProfilesMessage::EnterEdit);
        assert!(matches!(state.mode, ProfileMode::Edit(ref n) if n == "dev"));
        assert_eq!(state.form_name, "dev");

        // Change description
        state.form_field = FormField::Description;
        state.form_cursor = state.form_description.len();
        update(&mut state, ProfilesMessage::FormInput('!'));
        update(&mut state, ProfilesMessage::FormSubmit);
        assert!(matches!(state.mode, ProfileMode::List));
    }

    #[test]
    fn update_delete_profile() {
        let mut state = test_state();
        state.selected = 1; // "dev"
        update(&mut state, ProfilesMessage::EnterDelete);
        assert!(matches!(state.mode, ProfileMode::Delete(ref n) if n == "dev"));

        update(&mut state, ProfilesMessage::DeleteConfirm);
        assert!(matches!(state.mode, ProfileMode::List));
        assert_eq!(state.profile_count(), 1);
        assert!(!state
            .profiles_config
            .as_ref()
            .unwrap()
            .profiles
            .contains_key("dev"));
    }

    #[test]
    fn update_delete_cancel() {
        let mut state = test_state();
        state.selected = 1;
        update(&mut state, ProfilesMessage::EnterDelete);
        update(&mut state, ProfilesMessage::DeleteCancel);
        assert!(matches!(state.mode, ProfileMode::List));
        assert_eq!(state.profile_count(), 2);
    }

    #[test]
    fn update_form_cancel() {
        let mut state = test_state();
        update(&mut state, ProfilesMessage::EnterCreate);
        update(&mut state, ProfilesMessage::FormCancel);
        assert!(matches!(state.mode, ProfileMode::List));
    }

    #[test]
    fn update_form_backspace() {
        let mut state = test_state();
        update(&mut state, ProfilesMessage::EnterCreate);
        update(&mut state, ProfilesMessage::FormInput('a'));
        update(&mut state, ProfilesMessage::FormInput('b'));
        assert_eq!(state.form_name, "ab");
        update(&mut state, ProfilesMessage::FormBackspace);
        assert_eq!(state.form_name, "a");
    }

    #[test]
    fn update_form_next_field() {
        let mut state = test_state();
        update(&mut state, ProfilesMessage::EnterCreate);
        assert_eq!(state.form_field, FormField::Name);
        update(&mut state, ProfilesMessage::FormNextField);
        assert_eq!(state.form_field, FormField::Description);
        update(&mut state, ProfilesMessage::FormNextField);
        assert_eq!(state.form_field, FormField::Name);
    }

    #[test]
    fn update_create_empty_name_fails() {
        let mut state = test_state();
        update(&mut state, ProfilesMessage::EnterCreate);
        update(&mut state, ProfilesMessage::FormSubmit);
        assert!(state.error_message.is_some());
        assert!(matches!(state.mode, ProfileMode::Create));
    }

    // -- Render tests --

    #[test]
    fn render_list_mode() {
        let state = test_state();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_empty_list() {
        let state = ProfilesState::new();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_form_mode() {
        let mut state = test_state();
        state.mode = ProfileMode::Create;
        state.form_name = "test".to_string();
        state.form_cursor = 4;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_delete_mode() {
        let mut state = test_state();
        state.mode = ProfileMode::Delete("dev".to_string());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }

    #[test]
    fn render_small_area_does_not_panic() {
        let state = test_state();
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f.buffer_mut(), area);
            })
            .unwrap();
    }
}
