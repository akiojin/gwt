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
    pub summary: Rect,
    pub env: Rect,
    pub disabled: Rect,
    pub preview: Rect,
    pub list_hint: Rect,
    pub list_content: Rect,
    pub env_hint: Rect,
    pub env_content: Rect,
    pub disabled_hint: Rect,
    pub disabled_content: Rect,
    pub preview_hint: Rect,
    pub preview_content: Rect,
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
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Min(5),
        ])
        .split(chunks[1]);
    let (list_hint, list_content) = split_with_hint(chunks[0]);
    let (env_hint, env_content) = split_with_hint(sections[1]);
    let (disabled_hint, disabled_content) = split_with_hint(sections[2]);
    let (preview_hint, preview_content) = split_with_hint(sections[3]);

    LayoutAreas {
        list: chunks[0],
        detail: chunks[1],
        summary: sections[0],
        env: sections[1],
        disabled: sections[2],
        preview: sections[3],
        list_hint,
        list_content,
        env_hint,
        env_content,
        disabled_hint,
        disabled_content,
        preview_hint,
        preview_content,
    }
}

/// A single environment variable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVarItem {
    pub key: String,
    pub value: String,
}

/// Focus target inside the Profiles tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfilesFocus {
    #[default]
    ProfileList,
    EnvVars,
    DisabledEnv,
    Preview,
}

impl ProfilesFocus {
    fn next(self) -> Self {
        match self {
            Self::ProfileList => Self::EnvVars,
            Self::EnvVars => Self::DisabledEnv,
            Self::DisabledEnv => Self::Preview,
            Self::Preview => Self::ProfileList,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::ProfileList => Self::Preview,
            Self::EnvVars => Self::ProfileList,
            Self::DisabledEnv => Self::EnvVars,
            Self::Preview => Self::DisabledEnv,
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
        self.selected_profile()
            .and_then(|profile| profile.env_vars.get(self.env_selected))
    }

    /// Get the currently selected disabled OS environment variable, if any.
    pub fn selected_disabled_env(&self) -> Option<&str> {
        self.selected_profile()
            .and_then(|profile| profile.disabled_env.get(self.disabled_selected))
            .map(String::as_str)
    }

    /// Clamp all selection indices to the currently available data.
    pub fn clamp_selection(&mut self) {
        super::clamp_index(&mut self.selected, self.profiles.len());
        let env_len = self
            .selected_profile()
            .map(|profile| profile.env_vars.len())
            .unwrap_or(0);
        let disabled_len = self
            .selected_profile()
            .map(|profile| profile.disabled_env.len())
            .unwrap_or(0);
        super::clamp_index(&mut self.env_selected, env_len);
        super::clamp_index(&mut self.disabled_selected, disabled_len);
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
                ProfilesFocus::EnvVars => {
                    let len = state
                        .selected_profile()
                        .map(|profile| profile.env_vars.len())
                        .unwrap_or(0);
                    super::move_up(&mut state.env_selected, len);
                }
                ProfilesFocus::DisabledEnv => {
                    let len = state
                        .selected_profile()
                        .map(|profile| profile.disabled_env.len())
                        .unwrap_or(0);
                    super::move_up(&mut state.disabled_selected, len);
                }
                ProfilesFocus::Preview => {}
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
                ProfilesFocus::EnvVars => {
                    let len = state
                        .selected_profile()
                        .map(|profile| profile.env_vars.len())
                        .unwrap_or(0);
                    super::move_down(&mut state.env_selected, len);
                }
                ProfilesFocus::DisabledEnv => {
                    let len = state
                        .selected_profile()
                        .map(|profile| profile.disabled_env.len())
                        .unwrap_or(0);
                    super::move_down(&mut state.disabled_selected, len);
                }
                ProfilesFocus::Preview => {}
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
                ProfilesFocus::EnvVars => ProfileMode::CreateEnvVar,
                ProfilesFocus::DisabledEnv => ProfileMode::CreateDisabledEnv,
                ProfilesFocus::Preview => ProfileMode::List,
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
                ProfilesFocus::EnvVars => {
                    if let Some(env) = state.selected_env_var().cloned() {
                        state.input_key = env.key.clone();
                        state.input_value = env.value.clone();
                        state.active_field = 0;
                        state.mode = ProfileMode::EditEnvVar;
                    }
                }
                ProfilesFocus::DisabledEnv => {
                    if let Some(key) = state.selected_disabled_env() {
                        state.input_key = key.to_string();
                        state.active_field = 0;
                        state.mode = ProfileMode::EditDisabledEnv;
                    }
                }
                ProfilesFocus::Preview => {}
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
                ProfilesFocus::EnvVars if state.selected_env_var().is_some() => {
                    ProfileMode::ConfirmDeleteEnvVar
                }
                ProfilesFocus::DisabledEnv if state.selected_disabled_env().is_some() => {
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
            .title("Profile Detail")
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

    render_summary_block(state, profile, frame, areas.summary);
    render_env_vars_block(
        state,
        profile,
        frame,
        areas.env,
        areas.env_hint,
        areas.env_content,
    );
    render_disabled_env_block(
        state,
        profile,
        frame,
        areas.disabled,
        areas.disabled_hint,
        areas.disabled_content,
    );
    render_preview_block(
        state,
        profile,
        frame,
        areas.preview,
        areas.preview_hint,
        areas.preview_content,
    );
}

fn render_summary_block(
    state: &ProfilesState,
    profile: &ProfileItem,
    frame: &mut Frame,
    area: Rect,
) {
    let lines = vec![
        Line::from(vec![
            Span::styled("Name: ", theme::style::header()),
            Span::raw(profile.name.clone()),
        ]),
        Line::from(vec![
            Span::styled("Desc: ", theme::style::header()),
            Span::raw(if profile.description.is_empty() {
                "(none)".to_string()
            } else {
                profile.description.clone()
            }),
        ]),
        Line::from(vec![
            Span::styled("Active: ", theme::style::header()),
            Span::styled(
                if profile.active { "yes" } else { "no" },
                if profile.active {
                    theme::style::success_text()
                } else {
                    theme::style::muted_text()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Delete: ", theme::style::header()),
            Span::styled(
                if profile.deletable {
                    "allowed"
                } else {
                    "locked (default)"
                },
                if profile.deletable {
                    theme::style::text()
                } else {
                    theme::style::warning_text()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Guide: ", theme::style::header()),
            Span::raw("Enter activates | Tab moves to Environment."),
        ]),
    ];

    let (border_style, border_type) = theme::pane_border(state.focus == ProfilesFocus::ProfileList);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title("Profile Detail")
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .border_type(border_type),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_env_vars_block(
    state: &ProfilesState,
    profile: &ProfileItem,
    frame: &mut Frame,
    area: Rect,
    hint_area: Rect,
    content_area: Rect,
) {
    let (border_style, border_type) = theme::pane_border(state.focus == ProfilesFocus::EnvVars);
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
    let items: Vec<ListItem> = if profile.env_vars.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No profile-owned environment variables. Press n to add one.".to_string(),
            theme::style::muted_text(),
        )]))]
    } else {
        profile
            .env_vars
            .iter()
            .enumerate()
            .map(|(idx, env)| {
                let style = if state.focus == ProfilesFocus::EnvVars && idx == state.env_selected {
                    theme::style::selected_item()
                } else {
                    theme::style::text()
                };
                ListItem::new(Line::from(vec![Span::styled(
                    format!("{}={}", env.key, env.value),
                    style,
                )]))
            })
            .collect()
    };

    let list = List::new(items);
    let mut list_state = ListState::default();
    if !profile.env_vars.is_empty() {
        list_state.select(Some(state.env_selected));
    }
    frame.render_stateful_widget(list, content_area, &mut list_state);
}

fn render_disabled_env_block(
    state: &ProfilesState,
    profile: &ProfileItem,
    frame: &mut Frame,
    area: Rect,
    hint_area: Rect,
    content_area: Rect,
) {
    let (border_style, border_type) = theme::pane_border(state.focus == ProfilesFocus::DisabledEnv);
    frame.render_widget(
        Block::default()
            .title("Disabled OS Environment")
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(border_type),
        area,
    );
    frame.render_widget(
        Paragraph::new(disabled_env_hint_text(area.width))
            .style(theme::style::muted_text())
            .wrap(Wrap { trim: false }),
        hint_area,
    );
    let items: Vec<ListItem> = if profile.disabled_env.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No blocked OS environment variables. Press n to add one.".to_string(),
            theme::style::muted_text(),
        )]))]
    } else {
        profile
            .disabled_env
            .iter()
            .enumerate()
            .map(|(idx, key)| {
                let style = if state.focus == ProfilesFocus::DisabledEnv
                    && idx == state.disabled_selected
                {
                    theme::style::selected_item()
                } else {
                    theme::style::text()
                };
                ListItem::new(Line::from(vec![Span::styled(key.clone(), style)]))
            })
            .collect()
    };

    let list = List::new(items);
    let mut list_state = ListState::default();
    if !profile.disabled_env.is_empty() {
        list_state.select(Some(state.disabled_selected));
    }
    frame.render_stateful_widget(list, content_area, &mut list_state);
}

fn render_preview_block(
    state: &ProfilesState,
    profile: &ProfileItem,
    frame: &mut Frame,
    area: Rect,
    hint_area: Rect,
    content_area: Rect,
) {
    let (border_style, border_type) = theme::pane_border(state.focus == ProfilesFocus::Preview);
    let lines: Vec<Line> = if profile.merged_preview.is_empty() {
        vec![Line::from(vec![Span::styled(
            "No effective environment variables.".to_string(),
            theme::style::muted_text(),
        )])]
    } else {
        profile
            .merged_preview
            .iter()
            .map(|env| Line::from(format!("{}={}", env.key, env.value)))
            .collect()
    };

    frame.render_widget(
        Block::default()
            .title("Effective Environment")
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(border_type),
        area,
    );
    frame.render_widget(
        Paragraph::new(preview_hint_text(area.width))
            .style(theme::style::muted_text())
            .wrap(Wrap { trim: false }),
        hint_area,
    );
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        content_area,
    );
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
        "Enter/e edit | n add | d delete"
    } else {
        "↵/e | n | d"
    }
}

fn disabled_env_hint_text(width: u16) -> &'static str {
    if width >= 28 {
        "Enter/e edit | n add | d delete"
    } else {
        "↵/e | n | d"
    }
}

fn preview_hint_text(width: u16) -> &'static str {
    if width >= 28 {
        "Read-only OS env with profile overrides"
    } else {
        "Read-only OS env"
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
        assert_eq!(state.focus, ProfilesFocus::EnvVars);
        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.env_selected, 0);

        state.selected = 1;
        state.clamp_selection();
        update(&mut state, ProfilesMessage::MoveDown);
        assert_eq!(state.env_selected, 1);

        update(&mut state, ProfilesMessage::FocusRight);
        assert_eq!(state.focus, ProfilesFocus::DisabledEnv);
    }

    #[test]
    fn start_create_uses_focus_specific_modes() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();

        update(&mut state, ProfilesMessage::StartCreate);
        assert_eq!(state.mode, ProfileMode::CreateProfile);

        state.exit_mode();
        state.focus = ProfilesFocus::EnvVars;
        update(&mut state, ProfilesMessage::StartCreate);
        assert_eq!(state.mode, ProfileMode::CreateEnvVar);

        state.exit_mode();
        state.focus = ProfilesFocus::DisabledEnv;
        update(&mut state, ProfilesMessage::StartCreate);
        assert_eq!(state.mode, ProfileMode::CreateDisabledEnv);
    }

    #[test]
    fn start_edit_prefills_selected_item() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 1;
        state.focus = ProfilesFocus::EnvVars;

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
        assert!(text.contains("Disabled OS Environment"), "{text}");
        assert!(text.contains("Effective Environment"), "{text}");
        assert!(text.contains("locked (default)"), "{text}");
    }

    #[test]
    fn render_empty_panes_show_guided_help() {
        let mut state = ProfilesState::default();
        state.profiles = sample_profiles();
        state.selected = 1;
        state.profiles[1].env_vars.clear();
        state.profiles[1].disabled_env.clear();
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
        assert!(text.contains("Read-only OS env"), "{text}");
    }
}
