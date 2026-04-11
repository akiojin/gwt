//! Profiles management screen.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::theme;

#[derive(Debug, Clone, Copy)]
pub(crate) struct LayoutAreas {
    pub list: Rect,
    pub detail: Rect,
    pub env: Rect,
    pub list_hint: Rect,
    pub list_content: Rect,
    pub env_hint: Rect,
    pub env_content: Rect,
}

fn bordered_inner(area: Rect) -> Rect {
    Block::default().borders(Borders::ALL).inner(area)
}

fn split_with_hint(area: Rect) -> (Rect, Rect) {
    let inner = bordered_inner(area);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    (sections[0], sections[1])
}

pub(crate) fn layout_areas(area: Rect) -> LayoutAreas {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);
    let (list_hint, list_content) = split_with_hint(chunks[0]);
    let (env_hint, env_content) = split_with_hint(chunks[1]);

    LayoutAreas {
        list: chunks[0],
        detail: chunks[1],
        env: chunks[1],
        list_hint,
        list_content,
        env_hint,
        env_content,
    }
}

/// A single environment variable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVarItem {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileEnvRowKind {
    Base,
    Override,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEnvRow {
    pub key: String,
    pub value: Option<String>,
    pub kind: ProfileEnvRowKind,
}

/// Focus target inside the Profiles tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfilesFocus {
    #[default]
    ProfileList,
    Environment,
}

impl ProfilesFocus {
    fn next(self) -> Self {
        match self {
            Self::ProfileList => Self::Environment,
            Self::Environment => Self::ProfileList,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::ProfileList => Self::Environment,
            Self::Environment => Self::ProfileList,
        }
    }
}

/// Current mode of the profiles screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileMode {
    #[default]
    List,
    CreateProfile,
    EditProfile,
    CreateEnvVar,
    EditEnvVar,
    CreateDisabledEnv,
    EditDisabledEnv,
    ConfirmDeleteProfile,
    ConfirmDeleteEnvVar,
    ConfirmDeleteDisabledEnv,
}

impl ProfileMode {
    fn is_form(self) -> bool {
        matches!(
            self,
            Self::CreateProfile
                | Self::EditProfile
                | Self::CreateEnvVar
                | Self::EditEnvVar
                | Self::CreateDisabledEnv
                | Self::EditDisabledEnv
        )
    }

    fn field_count(self) -> usize {
        match self {
            Self::CreateProfile | Self::EditProfile => 2,
            Self::CreateEnvVar | Self::EditEnvVar => 2,
            Self::CreateDisabledEnv | Self::EditDisabledEnv => 1,
            _ => 0,
        }
    }
}

/// A single profile entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileItem {
    pub name: String,
    pub active: bool,
    pub env_count: usize,
    pub description: String,
    pub env_vars: Vec<EnvVarItem>,
    pub disabled_env: Vec<String>,
    pub env_rows: Vec<ProfileEnvRow>,
    pub merged_preview: Vec<EnvVarItem>,
    pub deletable: bool,
}

/// State for the profiles screen.
#[derive(Debug, Clone, Default)]
pub struct ProfilesState {
    pub(crate) profiles: Vec<ProfileItem>,
    pub(crate) selected: usize,
    pub(crate) env_selected: usize,
    pub(crate) disabled_selected: usize,
    pub(crate) focus: ProfilesFocus,
    pub(crate) mode: ProfileMode,
    pub(crate) input_name: String,
    pub(crate) input_description: String,
    pub(crate) input_key: String,
    pub(crate) input_value: String,
    /// Index of the active field in the current form.
    pub(crate) active_field: usize,
}

impl ProfilesState {
    /// Get the currently selected profile, if any.
    pub fn selected_profile(&self) -> Option<&ProfileItem> {
        self.profiles.get(self.selected)
    }

    /// Get the currently selected environment variable, if any.
    pub fn selected_env_var(&self) -> Option<&EnvVarItem> {
        let row = self.selected_env_row()?;
        if row.kind != ProfileEnvRowKind::Override {
            return None;
        }
        self.selected_profile()
            .and_then(|profile| profile.env_vars.iter().find(|env| env.key == row.key))
    }

    /// Get the currently selected environment row, if any.
    pub fn selected_env_row(&self) -> Option<&ProfileEnvRow> {
        self.selected_profile()
            .and_then(|profile| profile.env_rows.get(self.env_selected))
    }

    /// Get the currently selected disabled OS environment variable, if any.
    pub fn selected_disabled_env(&self) -> Option<&str> {
        self.selected_env_row()
            .filter(|row| row.kind == ProfileEnvRowKind::Disabled)
            .map(|row| row.key.as_str())
    }

    /// Clamp all selection indices to the currently available data.
    pub fn clamp_selection(&mut self) {
        super::clamp_index(&mut self.selected, self.profiles.len());
        let env_len = self
            .selected_profile()
            .map(|profile| profile.env_rows.len())
            .unwrap_or(0);
        super::clamp_index(&mut self.env_selected, env_len);
        self.disabled_selected = self.env_selected;
    }

    fn clear_form(&mut self) {
        self.input_name.clear();
        self.input_description.clear();
        self.input_key.clear();
        self.input_value.clear();
        self.active_field = 0;
    }

    fn exit_mode(&mut self) {
        self.clear_form();
        self.mode = ProfileMode::List;
    }
}

/// Messages specific to the profiles screen.
#[derive(Debug, Clone)]
pub enum ProfilesMessage {
    MoveUp,
    MoveDown,
    FocusLeft,
    FocusRight,
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
            if state.mode != ProfileMode::List {
                return;
            }
            match state.focus {
                ProfilesFocus::ProfileList => {
                    super::move_up(&mut state.selected, state.profiles.len());
                    state.clamp_selection();
                }
                ProfilesFocus::Environment => {
                    let len = state
                        .selected_profile()
                        .map(|profile| profile.env_rows.len())
                        .unwrap_or(0);
                    super::move_up(&mut state.env_selected, len);
                }
            }
        }
        ProfilesMessage::MoveDown => {
            if state.mode != ProfileMode::List {
                return;
            }
            match state.focus {
                ProfilesFocus::ProfileList => {
                    super::move_down(&mut state.selected, state.profiles.len());
                    state.clamp_selection();
                }
                ProfilesFocus::Environment => {
                    let len = state
                        .selected_profile()
                        .map(|profile| profile.env_rows.len())
                        .unwrap_or(0);
                    super::move_down(&mut state.env_selected, len);
                }
            }
        }
        ProfilesMessage::FocusLeft => {
            if state.mode == ProfileMode::List {
                state.focus = state.focus.prev();
            }
        }
        ProfilesMessage::FocusRight => {
            if state.mode == ProfileMode::List {
                state.focus = state.focus.next();
            }
        }
        ProfilesMessage::ToggleActive => {}
        ProfilesMessage::StartCreate => {
            if state.mode != ProfileMode::List {
                return;
            }
            state.clear_form();
            state.mode = match state.focus {
                ProfilesFocus::ProfileList => ProfileMode::CreateProfile,
                ProfilesFocus::Environment => ProfileMode::CreateEnvVar,
            };
        }
        ProfilesMessage::StartEdit => {
            if state.mode != ProfileMode::List {
                return;
            }
            match state.focus {
                ProfilesFocus::ProfileList => {
                    if let Some(profile) = state.selected_profile().cloned() {
                        state.input_name = profile.name.clone();
                        state.input_description = profile.description.clone();
                        state.active_field = 0;
                        state.mode = ProfileMode::EditProfile;
                    }
                }
                ProfilesFocus::Environment => {
                    if let Some(row) = state.selected_env_row().cloned() {
                        state.input_key = row.key.clone();
                        state.input_value = row.value.unwrap_or_default();
                        state.active_field = 0;
                        state.mode = match row.kind {
                            ProfileEnvRowKind::Base => ProfileMode::CreateEnvVar,
                            ProfileEnvRowKind::Override => ProfileMode::EditEnvVar,
                            ProfileEnvRowKind::Disabled => ProfileMode::EditDisabledEnv,
                        };
                    }
                }
            }
        }
        ProfilesMessage::StartDelete => {
            if state.mode != ProfileMode::List {
                return;
            }
            state.mode = match state.focus {
                ProfilesFocus::ProfileList if state.selected_profile().is_some() => {
                    ProfileMode::ConfirmDeleteProfile
                }
                ProfilesFocus::Environment if state.selected_env_var().is_some() => {
                    ProfileMode::ConfirmDeleteEnvVar
                }
                ProfilesFocus::Environment if state.selected_disabled_env().is_some() => {
                    ProfileMode::ConfirmDeleteDisabledEnv
                }
                _ => ProfileMode::List,
            };
        }
        ProfilesMessage::Confirm => state.exit_mode(),
        ProfilesMessage::Cancel => state.exit_mode(),
        ProfilesMessage::InputChar(ch) => {
            if !state.mode.is_form() {
                return;
            }
            match state.mode {
                ProfileMode::CreateProfile | ProfileMode::EditProfile => {
                    if state.active_field == 0 {
                        state.input_name.push(ch);
                    } else {
                        state.input_description.push(ch);
                    }
                }
                ProfileMode::CreateEnvVar | ProfileMode::EditEnvVar => {
                    if state.active_field == 0 {
                        state.input_key.push(ch);
                    } else {
                        state.input_value.push(ch);
                    }
                }
                ProfileMode::CreateDisabledEnv | ProfileMode::EditDisabledEnv => {
                    state.input_key.push(ch);
                }
                _ => {}
            }
        }
        ProfilesMessage::Backspace => {
            if !state.mode.is_form() {
                return;
            }
            match state.mode {
                ProfileMode::CreateProfile | ProfileMode::EditProfile => {
                    if state.active_field == 0 {
                        state.input_name.pop();
                    } else {
                        state.input_description.pop();
                    }
                }
                ProfileMode::CreateEnvVar | ProfileMode::EditEnvVar => {
                    if state.active_field == 0 {
                        state.input_key.pop();
                    } else {
                        state.input_value.pop();
                    }
                }
                ProfileMode::CreateDisabledEnv | ProfileMode::EditDisabledEnv => {
                    state.input_key.pop();
                }
                _ => {}
            }
        }
        ProfilesMessage::NextField => {
            if state.mode.is_form() {
                let field_count = state.mode.field_count();
                if field_count > 0 {
                    state.active_field = (state.active_field + 1) % field_count;
                }
            }
        }
    }
}

/// Render the profiles screen.
pub fn render(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    let areas = layout_areas(area);

    render_list(state, frame, areas);
    match state.mode {
        ProfileMode::List => render_detail(state, frame, areas),
        ProfileMode::CreateProfile
        | ProfileMode::EditProfile
        | ProfileMode::CreateEnvVar
        | ProfileMode::EditEnvVar
        | ProfileMode::CreateDisabledEnv
        | ProfileMode::EditDisabledEnv => render_form(state, frame, areas.detail),
        ProfileMode::ConfirmDeleteProfile
        | ProfileMode::ConfirmDeleteEnvVar
        | ProfileMode::ConfirmDeleteDisabledEnv => {
            render_confirm_delete(state, frame, areas.detail)
        }
    }
}

/// Render the profile list view.
fn render_list(state: &ProfilesState, frame: &mut Frame, areas: LayoutAreas) {
    let (border_style, border_type) = theme::pane_border(state.focus == ProfilesFocus::ProfileList);
    let block = Block::default()
        .title("Profiles")
        .borders(Borders::ALL)
        .border_style(border_style)
        .border_type(border_type);
    frame.render_widget(block, areas.list);
    frame.render_widget(
        Paragraph::new(list_hint_text(areas.list_hint.width))
            .style(theme::style::muted_text())
            .wrap(Wrap { trim: false }),
        areas.list_hint,
    );

    if state.profiles.is_empty() {
        frame.render_widget(
            Paragraph::new("No profiles loaded. Press n to create one.")
                .style(theme::style::muted_text())
                .wrap(Wrap { trim: false }),
            areas.list_content,
        );
        return;
    }

    let items: Vec<ListItem> = state
        .profiles
        .iter()
        .enumerate()
        .map(|(idx, profile)| {
            let style = if idx == state.selected {
                theme::style::selected_item()
            } else {
                theme::style::text()
            };
            let active_marker = if profile.active { "[*]" } else { "[ ]" };
            let mut spans = vec![
                Span::styled(
                    format!("{active_marker} "),
                    if profile.active {
                        theme::style::success_text()
                    } else {
                        Style::default().fg(theme::color::SURFACE)
                    },
                ),
                Span::styled(profile.name.clone(), style),
                Span::styled(
                    format!("  ({} env vars)", profile.env_count),
                    Style::default().fg(theme::color::FOCUS),
                ),
            ];
            if !profile.deletable {
                spans.push(Span::styled(
                    "  [default]".to_string(),
                    theme::style::warning_text(),
                ));
            }
            if !profile.description.is_empty() {
                spans.push(Span::styled(
                    format!(" - {}", profile.description),
                    Style::default().fg(theme::color::SURFACE),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ListState::default();
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, areas.list_content, &mut list_state);
}

fn render_detail(state: &ProfilesState, frame: &mut Frame, areas: LayoutAreas) {
    let Some(profile) = state.selected_profile() else {
        let block = Block::default()
            .title("Environment")
            .borders(Borders::ALL)
            .border_type(theme::border::default());
        frame.render_widget(
            Paragraph::new("No profile selected. Press n to create a profile.")
                .block(block)
                .style(theme::style::muted_text()),
            areas.detail,
        );
        return;
    };

    render_environment_block(
        state,
        profile,
        frame,
        areas.env,
        areas.env_hint,
        areas.env_content,
    );
}

fn render_environment_block(
    state: &ProfilesState,
    profile: &ProfileItem,
    frame: &mut Frame,
    area: Rect,
    hint_area: Rect,
    content_area: Rect,
) {
    let (border_style, border_type) = theme::pane_border(state.focus == ProfilesFocus::Environment);
    frame.render_widget(
        Block::default()
            .title("Environment")
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(border_type),
        area,
    );
    frame.render_widget(
        Paragraph::new(env_hint_text(area.width))
            .style(theme::style::muted_text())
            .wrap(Wrap { trim: false }),
        hint_area,
    );
    let items: Vec<ListItem> = if profile.env_rows.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No environment rows. Press n to add one.".to_string(),
            theme::style::muted_text(),
        )]))]
    } else {
        profile
            .env_rows
            .iter()
            .enumerate()
            .map(|(idx, env)| {
                let is_selected =
                    state.focus == ProfilesFocus::Environment && idx == state.env_selected;
                ListItem::new(Line::from(vec![Span::styled(
                    format!(
                        "{}={}",
                        env.key,
                        env.value.as_deref().unwrap_or("<missing>")
                    ),
                    env_row_style(env, is_selected),
                )]))
            })
            .collect()
    };

    let list = List::new(items);
    let mut list_state = ListState::default();
    if !profile.env_rows.is_empty() {
        list_state.select(Some(state.env_selected));
    }
    frame.render_stateful_widget(list, content_area, &mut list_state);
}

fn env_row_style(row: &ProfileEnvRow, is_selected: bool) -> Style {
    let mut style = match row.kind {
        ProfileEnvRowKind::Base => theme::style::text(),
        ProfileEnvRowKind::Override => theme::style::warning_text(),
        ProfileEnvRowKind::Disabled => {
            theme::style::muted_text().add_modifier(Modifier::CROSSED_OUT)
        }
    };
    if is_selected {
        style = style.bg(theme::color::SURFACE).add_modifier(Modifier::BOLD);
    }
    style
}

fn list_hint_text(width: u16) -> &'static str {
    if width >= 24 {
        "Tab panes | Enter activate | n/e/d"
    } else {
        "Tab | ↵ | n/e/d"
    }
}

fn env_hint_text(width: u16) -> &'static str {
    if width >= 28 {
        "Enter/e edit | n add | d delete/restore"
    } else {
        "↵/e | n | d"
    }
}

fn render_form(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    let title = match state.mode {
        ProfileMode::CreateProfile => "Create Profile",
        ProfileMode::EditProfile => "Edit Profile",
        ProfileMode::CreateEnvVar => "Add Environment Variable",
        ProfileMode::EditEnvVar => "Edit Environment Variable",
        ProfileMode::CreateDisabledEnv => "Add Disabled OS Environment",
        ProfileMode::EditDisabledEnv => "Edit Disabled OS Environment",
        _ => "Profile Form",
    };

    let constraints = match state.mode {
        ProfileMode::CreateProfile | ProfileMode::EditProfile => {
            vec![
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(0),
            ]
        }
        ProfileMode::CreateEnvVar | ProfileMode::EditEnvVar => {
            vec![
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(0),
            ]
        }
        ProfileMode::CreateDisabledEnv | ProfileMode::EditDisabledEnv => {
            vec![
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Min(0),
            ]
        }
        _ => vec![Constraint::Min(0)],
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    match state.mode {
        ProfileMode::CreateProfile | ProfileMode::EditProfile => {
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
            frame.render_widget(
                Paragraph::new("Tab: next field | Enter: confirm | Esc: cancel")
                    .style(theme::style::muted_text()),
                chunks[2],
            );
        }
        ProfileMode::CreateEnvVar | ProfileMode::EditEnvVar => {
            render_form_field(
                title,
                &format!("Key: {}", state.input_key),
                state.active_field == 0,
                frame,
                chunks[0],
            );
            render_form_field(
                "Value",
                &state.input_value,
                state.active_field == 1,
                frame,
                chunks[1],
            );
            frame.render_widget(
                Paragraph::new("Tab: next field | Enter: confirm | Esc: cancel")
                    .style(theme::style::muted_text()),
                chunks[2],
            );
        }
        ProfileMode::CreateDisabledEnv | ProfileMode::EditDisabledEnv => {
            render_form_field(
                title,
                &format!("Key: {}", state.input_key),
                true,
                frame,
                chunks[0],
            );
            frame.render_widget(
                Paragraph::new("Enter: confirm | Esc: cancel").style(theme::style::muted_text()),
                chunks[1],
            );
        }
        _ => {}
    }
}

fn render_form_field(title: &str, value: &str, is_active: bool, frame: &mut Frame, area: Rect) {
    let (border_style, border_type) = theme::pane_border(is_active);
    let text_style = if is_active {
        Style::default()
            .fg(theme::color::ACTIVE)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::color::TEXT_PRIMARY)
    };
    let display = if is_active {
        format!("{value}_")
    } else {
        value.to_string()
    };
    frame.render_widget(
        Paragraph::new(display)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .border_type(border_type),
            )
            .style(text_style),
        area,
    );
}

fn render_confirm_delete(state: &ProfilesState, frame: &mut Frame, area: Rect) {
    let (title, target) = match state.mode {
        ProfileMode::ConfirmDeleteProfile => (
            "Delete Profile",
            state.selected_profile().map(|profile| profile.name.clone()),
        ),
        ProfileMode::ConfirmDeleteEnvVar => (
            "Delete Environment Variable",
            state.selected_env_var().map(|env| env.key.clone()),
        ),
        ProfileMode::ConfirmDeleteDisabledEnv => (
            "Delete Disabled OS Environment",
            state.selected_disabled_env().map(str::to_string),
        ),
        _ => ("Delete", None),
    };

    frame.render_widget(
        Paragraph::new(format!(
            "Delete \"{}\"?\n\nEnter: confirm | Esc: cancel",
            target.unwrap_or_else(|| "unknown".to_string())
        ))
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(theme::border::modal())
                .border_style(theme::style::error_text()),
        )
        .style(theme::style::error_text())
        .wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};
    use ratatui::Terminal;

    fn sample_profiles() -> Vec<ProfileItem> {
        vec![
            ProfileItem {
                name: "default".to_string(),
                active: true,
                env_count: 1,
                description: "Default profile".to_string(),
                env_vars: vec![EnvVarItem {
                    key: "TERM".to_string(),
                    value: "xterm-256color".to_string(),
                }],
                disabled_env: vec!["SECRET".to_string()],
                env_rows: vec![
                    ProfileEnvRow {
                        key: "PATH".to_string(),
                        value: Some("/bin".to_string()),
                        kind: ProfileEnvRowKind::Base,
                    },
                    ProfileEnvRow {
                        key: "SECRET".to_string(),
                        value: Some("hidden".to_string()),
                        kind: ProfileEnvRowKind::Disabled,
                    },
                    ProfileEnvRow {
                        key: "TERM".to_string(),
                        value: Some("xterm-256color".to_string()),
                        kind: ProfileEnvRowKind::Override,
                    },
                ],
                merged_preview: vec![
                    EnvVarItem {
                        key: "PATH".to_string(),
                        value: "/bin".to_string(),
                    },
                    EnvVarItem {
                        key: "TERM".to_string(),
                        value: "xterm-256color".to_string(),
                    },
                ],
                deletable: false,
            },
            ProfileItem {
                name: "staging".to_string(),
                active: false,
                env_count: 2,
                description: "Staging env".to_string(),
                env_vars: vec![
                    EnvVarItem {
                        key: "API_URL".to_string(),
                        value: "https://staging".to_string(),
                    },
                    EnvVarItem {
                        key: "FEATURE_FLAG".to_string(),
                        value: "1".to_string(),
                    },
                ],
                disabled_env: vec![],
                env_rows: vec![
                    ProfileEnvRow {
                        key: "API_URL".to_string(),
                        value: Some("https://staging".to_string()),
                        kind: ProfileEnvRowKind::Override,
                    },
                    ProfileEnvRow {
                        key: "FEATURE_FLAG".to_string(),
                        value: Some("1".to_string()),
                        kind: ProfileEnvRowKind::Override,
                    },
                    ProfileEnvRow {
                        key: "PATH".to_string(),
                        value: Some("/usr/bin".to_string()),
                        kind: ProfileEnvRowKind::Base,
                    },
                ],
                merged_preview: vec![EnvVarItem {
                    key: "API_URL".to_string(),
                    value: "https://staging".to_string(),
                }],
                deletable: true,
            },
        ]
    }

    #[test]
    fn default_state() {
        let state = ProfilesState::default();
        assert!(state.profiles.is_empty());
        assert_eq!(state.selected, 0);
        assert_eq!(state.env_selected, 0);
        assert_eq!(state.disabled_selected, 0);
        assert_eq!(state.focus, ProfilesFocus::ProfileList);
        assert_eq!(state.mode, ProfileMode::List);
    }

    #[test]
    fn focus_and_selection_follow_current_section() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        update(&mut state, ProfilesMessage::FocusRight);
        assert_eq!(state.focus, ProfilesFocus::Environment);
        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.env_selected, 1);

        state.selected = 1;
        state.clamp_selection();
        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.env_selected, 2);

        update(&mut state, ProfilesMessage::FocusRight);
        assert_eq!(state.focus, ProfilesFocus::ProfileList);
    }

    #[test]
    fn start_create_uses_focus_specific_modes() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        update(&mut state, ProfilesMessage::StartCreate);
        assert_eq!(state.mode, ProfileMode::CreateProfile);

        state.exit_mode();
        state.focus = ProfilesFocus::Environment;
        update(&mut state, ProfilesMessage::StartCreate);
        assert_eq!(state.mode, ProfileMode::CreateEnvVar);
    }

    #[test]
    fn start_edit_prefills_selected_item() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 1;
        state.focus = ProfilesFocus::Environment;

        update(&mut state, ProfilesMessage::StartEdit);
        assert_eq!(state.mode, ProfileMode::EditEnvVar);
        assert_eq!(state.input_key, "API_URL");
        assert_eq!(state.input_value, "https://staging");
    }

    #[test]
    fn next_field_cycles_inside_active_form() {
        let mut state = ProfilesState::default();
        state.mode = ProfileMode::CreateEnvVar;

        update(&mut state, ProfilesMessage::NextField);
        assert_eq!(state.active_field, 1);
        update(&mut state, ProfilesMessage::NextField);
        assert_eq!(state.active_field, 0);
    }

    #[test]
    fn render_shows_detail_sections_and_default_lock() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let text = buf
            .content
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect::<String>();
        assert!(text.contains("Profiles"), "{text}");
        assert!(text.contains("Environment"), "{text}");
        assert!(!text.contains("Profile Detail"), "{text}");
        assert!(!text.contains("Name:"), "{text}");
        assert!(!text.contains("Desc:"), "{text}");
        assert!(!text.contains("Disabled OS Environment"), "{text}");
        assert!(!text.contains("Effective Environment"), "{text}");
        assert!(text.contains("[default]"), "{text}");
    }

    #[test]
    fn render_empty_panes_show_guided_help() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 1;
        state.profiles[1].env_vars.clear();
        state.profiles[1].disabled_env.clear();
        state.profiles[1].env_rows.clear();
        state.profiles[1].merged_preview.clear();

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let text = buf
            .content
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect::<String>();
        assert!(text.contains("Press n to add"), "{text}");
        assert!(!text.contains("Read-only OS env"), "{text}");
    }

    #[test]
    fn render_unified_environment_list_marks_override_and_disabled_rows() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.focus = ProfilesFocus::ProfileList;

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let (override_x, override_y) =
            find_text(&buf, "TERM=xterm-256color").expect("override row");
        let override_cell = &buf[(override_x, override_y)];
        assert_eq!(override_cell.fg, Color::Yellow);

        let (disabled_x, disabled_y) = find_text(&buf, "SECRET=hidden").expect("disabled row");
        let disabled_cell = &buf[(disabled_x, disabled_y)];
        assert!(disabled_cell.modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn layout_uses_entire_right_pane_for_environment_block() {
        let areas = layout_areas(Rect::new(0, 0, 120, 30));

        assert_eq!(areas.env, areas.detail);
    }

    fn find_text(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
        for y in buf.area.y..buf.area.bottom() {
            let line = (buf.area.x..buf.area.right())
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>();
            if let Some(start) = line.find(needle) {
                return Some((buf.area.x + start as u16, y));
            }
        }
        None
    }
}
