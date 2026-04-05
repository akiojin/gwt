//! Settings management screen.

use std::path::PathBuf;

use gwt_agent::{custom::CustomAgentType, CustomCodingAgent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::theme;

use gwt_config::{ConfigError, Settings, VoiceConfig};
use gwt_skills::assets::CLAUDE_SKILLS;

use crate::custom_agents::{
    load_stored_custom_agents_from_path, save_stored_custom_agents_to_path, StoredCustomAgent,
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
    Skills,
    Voice,
}

impl SettingsCategory {
    /// All categories in display order.
    pub const ALL: [SettingsCategory; 8] = [
        SettingsCategory::General,
        SettingsCategory::Worktree,
        SettingsCategory::Agent,
        SettingsCategory::CustomAgents,
        SettingsCategory::Environment,
        SettingsCategory::Ai,
        SettingsCategory::Skills,
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
            Self::Skills => "Skills",
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

const CUSTOM_AGENT_LABEL: &str = "Agent";
const CUSTOM_AGENT_ID_LABEL: &str = "ID";
const CUSTOM_AGENT_DISPLAY_NAME_LABEL: &str = "Display name";
const CUSTOM_AGENT_TYPE_LABEL: &str = "Type";
const CUSTOM_AGENT_COMMAND_LABEL: &str = "Command";
const CUSTOM_AGENT_ADD_LABEL: &str = "Add agent";
const CUSTOM_AGENT_DELETE_LABEL: &str = "Delete agent";

/// Field type for a setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Bool,
    Path,
    Choice,
    Action,
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
    pub(crate) config_path_override: Option<PathBuf>,
    custom_agents: CustomAgentsState,
}

#[derive(Debug, Clone, Default)]
struct CustomAgentsState {
    agents: Vec<StoredCustomAgent>,
    selected_agent: usize,
}

impl CustomAgentsState {
    fn selected_agent(&self) -> Option<&StoredCustomAgent> {
        self.agents.get(self.selected_agent)
    }
}

impl SettingsState {
    /// Get the currently selected field, if any.
    pub fn selected_field(&self) -> Option<&SettingField> {
        self.fields.get(self.selected)
    }

    /// Load fields for the current category.
    pub fn load_category_fields(&mut self) {
        self.save_error = None;
        let settings = self.load_settings_snapshot();
        if self.category == SettingsCategory::CustomAgents {
            self.load_custom_agents_fields();
        } else {
            self.fields = fields_for_category_with_settings(self.category, settings.as_ref());
        }
        self.selected = 0;
        self.editing = false;
        self.edit_buffer.clear();
    }

    fn load_settings_snapshot(&self) -> Option<Settings> {
        self.config_path_override
            .as_deref()
            .and_then(|path| Settings::load_from_path(path).ok())
            .or_else(|| {
                if self.category == SettingsCategory::Voice {
                    Settings::load().ok()
                } else {
                    None
                }
            })
    }

    fn load_custom_agents_fields(&mut self) {
        let Some(path) = self.config_path() else {
            self.custom_agents = CustomAgentsState::default();
            sync_custom_agent_fields(self);
            return;
        };

        match load_stored_custom_agents_from_path(&path) {
            Ok(agents) => {
                let selected_agent = self
                    .custom_agents
                    .selected_agent
                    .min(agents.len().saturating_sub(1));
                self.custom_agents = CustomAgentsState {
                    agents,
                    selected_agent,
                };
            }
            Err(err) => {
                self.custom_agents = CustomAgentsState::default();
                self.save_error = Some(err);
            }
        }
        sync_custom_agent_fields(self);
    }

    fn config_path(&self) -> Option<PathBuf> {
        self.config_path_override
            .clone()
            .or_else(Settings::global_config_path)
    }
}

/// Return default fields for a given category.
pub fn fields_for_category(category: SettingsCategory) -> Vec<SettingField> {
    fields_for_category_with_settings(category, None)
}

/// Return default fields for a given category, optionally using live settings
/// for categories that should reflect persisted configuration.
pub fn fields_for_category_with_settings(
    category: SettingsCategory,
    settings: Option<&Settings>,
) -> Vec<SettingField> {
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
                label: CUSTOM_AGENT_LABEL.to_string(),
                value: "(none)".to_string(),
                field_type: FieldType::Choice,
            },
            SettingField {
                label: CUSTOM_AGENT_ADD_LABEL.to_string(),
                value: "Create new custom agent".to_string(),
                field_type: FieldType::Action,
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
        SettingsCategory::Skills => bundled_skill_fields(),
        SettingsCategory::Voice => {
            let voice = settings.map(|s| &s.voice);
            voice_fields(voice)
        }
    }
}

/// Build a read-only display of bundled skill count.
pub fn bundled_skill_fields() -> Vec<SettingField> {
    let count = CLAUDE_SKILLS.dirs().count();
    vec![SettingField {
        label: "Bundled skills".to_string(),
        value: count.to_string(),
        field_type: FieldType::Text,
    }]
}

/// Build the Voice settings fields from the persisted voice config.
pub fn voice_fields(voice: Option<&VoiceConfig>) -> Vec<SettingField> {
    let voice = voice.cloned().unwrap_or_default();
    vec![
        SettingField {
            label: "Model path".to_string(),
            value: voice
                .model_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            field_type: FieldType::Path,
        },
        SettingField {
            label: "Hotkey".to_string(),
            value: voice.hotkey,
            field_type: FieldType::Text,
        },
        SettingField {
            label: "Input device".to_string(),
            value: voice.input_device,
            field_type: FieldType::Text,
        },
        SettingField {
            label: "Language".to_string(),
            value: voice.language,
            field_type: FieldType::Text,
        },
        SettingField {
            label: "Enabled".to_string(),
            value: voice.enabled.to_string(),
            field_type: FieldType::Bool,
        },
    ]
}

fn custom_agent_fields(state: &CustomAgentsState) -> Vec<SettingField> {
    let selected = state.selected_agent();
    let summary = if let Some(agent) = selected {
        format!(
            "{} ({}/{})",
            agent.agent.display_name,
            state.selected_agent + 1,
            state.agents.len()
        )
    } else {
        "(none)".to_string()
    };
    let id = selected
        .map(|agent| agent.agent.id.clone())
        .unwrap_or_default();
    let display_name = selected
        .map(|agent| agent.agent.display_name.clone())
        .unwrap_or_default();
    let agent_type = selected
        .map(|agent| custom_agent_type_label(agent.agent.agent_type).to_string())
        .unwrap_or_else(|| custom_agent_type_label(CustomAgentType::Command).to_string());
    let command = selected
        .map(|agent| agent.agent.command.clone())
        .unwrap_or_default();

    vec![
        SettingField {
            label: CUSTOM_AGENT_LABEL.to_string(),
            value: summary,
            field_type: FieldType::Choice,
        },
        SettingField {
            label: CUSTOM_AGENT_ID_LABEL.to_string(),
            value: id,
            field_type: FieldType::Text,
        },
        SettingField {
            label: CUSTOM_AGENT_DISPLAY_NAME_LABEL.to_string(),
            value: display_name,
            field_type: FieldType::Text,
        },
        SettingField {
            label: CUSTOM_AGENT_TYPE_LABEL.to_string(),
            value: agent_type,
            field_type: FieldType::Choice,
        },
        SettingField {
            label: CUSTOM_AGENT_COMMAND_LABEL.to_string(),
            value: command,
            field_type: FieldType::Text,
        },
        SettingField {
            label: CUSTOM_AGENT_ADD_LABEL.to_string(),
            value: "Create new custom agent".to_string(),
            field_type: FieldType::Action,
        },
        SettingField {
            label: CUSTOM_AGENT_DELETE_LABEL.to_string(),
            value: if selected.is_some() {
                "Remove selected custom agent".to_string()
            } else {
                "No custom agent selected".to_string()
            },
            field_type: FieldType::Action,
        },
    ]
}

fn sync_custom_agent_fields(state: &mut SettingsState) {
    state.fields = custom_agent_fields(&state.custom_agents);
    if !state.fields.is_empty() {
        state.selected = state.selected.min(state.fields.len() - 1);
    }
}

fn custom_agent_type_label(agent_type: CustomAgentType) -> &'static str {
    match agent_type {
        CustomAgentType::Command => "command",
        CustomAgentType::Path => "path",
        CustomAgentType::Bunx => "bunx",
    }
}

fn next_custom_agent_type(agent_type: CustomAgentType) -> CustomAgentType {
    match agent_type {
        CustomAgentType::Command => CustomAgentType::Path,
        CustomAgentType::Path => CustomAgentType::Bunx,
        CustomAgentType::Bunx => CustomAgentType::Command,
    }
}

fn next_custom_agent_id(existing: &[StoredCustomAgent]) -> String {
    let base = "custom-agent";
    if !existing.iter().any(|agent| agent.agent.id == base) {
        return base.to_string();
    }

    let mut index = 2usize;
    loop {
        let candidate = format!("{base}-{index}");
        if !existing.iter().any(|agent| agent.agent.id == candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn default_custom_agent(existing: &[StoredCustomAgent]) -> StoredCustomAgent {
    let id = next_custom_agent_id(existing);
    StoredCustomAgent::new(CustomCodingAgent {
        id: id.clone(),
        display_name: format!("Custom Agent {}", existing.len() + 1),
        agent_type: CustomAgentType::Command,
        command: id,
        default_args: Vec::new(),
        mode_args: None,
        env: std::collections::HashMap::new(),
    })
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

fn collect_voice_config_from_fields(state: &SettingsState) -> VoiceConfig {
    let mut voice = VoiceConfig::default();

    for field in &state.fields {
        match field.label.as_str() {
            "Model path" => {
                voice.model_path = if field.value.trim().is_empty() {
                    None
                } else {
                    Some(std::path::PathBuf::from(&field.value))
                };
            }
            "Hotkey" => {
                voice.hotkey = if field.value.trim().is_empty() {
                    "Ctrl+G,v".to_string()
                } else {
                    field.value.clone()
                };
            }
            "Input device" => {
                voice.input_device = if field.value.trim().is_empty() {
                    "system_default".to_string()
                } else {
                    field.value.clone()
                };
            }
            "Language" => {
                voice.language = if field.value.trim().is_empty() {
                    "auto".to_string()
                } else {
                    field.value.clone()
                };
            }
            "Enabled" => {
                voice.enabled = field.value == "true";
            }
            _ => {}
        }
    }

    voice
}

/// Persist current settings fields to gwt-config's global config.
///
/// Reads the current global Settings, applies matching fields from the TUI state,
/// and writes back. Returns an error string on failure.
fn save_settings_to_config(state: &SettingsState) -> Result<(), String> {
    if let Some(path) = state.config_path_override.as_deref() {
        return save_settings_to_path(state, path);
    }
    if state.category == SettingsCategory::CustomAgents {
        let path = Settings::global_config_path()
            .ok_or_else(|| "failed to resolve ~/.gwt/config.toml".to_string())?;
        return save_stored_custom_agents_to_path(&path, &state.custom_agents.agents);
    }

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
                        settings.worktree_root = Some(std::path::PathBuf::from(&field.value));
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
                (SettingsCategory::Voice, "Model path")
                | (SettingsCategory::Voice, "Hotkey")
                | (SettingsCategory::Voice, "Input device")
                | (SettingsCategory::Voice, "Language")
                | (SettingsCategory::Voice, "Enabled") => {
                    settings.voice = collect_voice_config_from_fields(state);
                    settings
                        .voice
                        .validate()
                        .map_err(|e| ConfigError::ValidationError {
                            reason: e.to_string(),
                        })?;
                }
                _ => {} // Other fields have no backend mapping yet
            }
        }
        Ok(())
    })
    .map_err(|e| format!("{e}"))
}

/// Save settings to a specific TOML file path.
pub fn save_settings_to_path(state: &SettingsState, path: &std::path::Path) -> Result<(), String> {
    if state.category == SettingsCategory::CustomAgents {
        return save_stored_custom_agents_to_path(path, &state.custom_agents.agents);
    }

    let mut settings = Settings::load_from_path(path).unwrap_or_default();

    for field in &state.fields {
        match (state.category, field.label.as_str()) {
            (SettingsCategory::General, "Log level") => {
                settings.debug = field.value == "debug";
            }
            (SettingsCategory::Worktree, "Default path") => {
                if field.value.is_empty() || field.value == "~/.gwt/worktrees" {
                    settings.worktree_root = None;
                } else {
                    settings.worktree_root = Some(std::path::PathBuf::from(&field.value));
                }
            }
            (SettingsCategory::Agent, "Default agent") => {
                if field.value.is_empty() {
                    settings.agent.default_agent = None;
                } else {
                    settings.agent.default_agent = Some(field.value.clone());
                }
            }
            (SettingsCategory::Voice, "Model path")
            | (SettingsCategory::Voice, "Hotkey")
            | (SettingsCategory::Voice, "Input device")
            | (SettingsCategory::Voice, "Language")
            | (SettingsCategory::Voice, "Enabled") => {
                settings.voice = collect_voice_config_from_fields(state);
                settings.voice.validate().map_err(|e| e.to_string())?;
                break;
            }
            _ => {}
        }
    }

    settings.save(path).map_err(|e| e.to_string())
}

fn persist_custom_agents_update<F>(state: &mut SettingsState, mutate: F)
where
    F: FnOnce(&mut CustomAgentsState),
{
    let Some(path) = state.config_path() else {
        state.save_error = Some("failed to resolve ~/.gwt/config.toml".to_string());
        return;
    };

    let mut next = state.custom_agents.clone();
    mutate(&mut next);
    match save_stored_custom_agents_to_path(&path, &next.agents) {
        Ok(()) => {
            state.custom_agents = next;
            state.save_error = None;
            sync_custom_agent_fields(state);
        }
        Err(err) => {
            state.save_error = Some(err);
            sync_custom_agent_fields(state);
        }
    }
}

fn start_custom_agent_interaction(state: &mut SettingsState) {
    let Some(field) = state.fields.get(state.selected) else {
        return;
    };

    match field.label.as_str() {
        CUSTOM_AGENT_LABEL => {
            if !state.custom_agents.agents.is_empty() {
                state.custom_agents.selected_agent =
                    (state.custom_agents.selected_agent + 1) % state.custom_agents.agents.len();
                state.save_error = None;
                sync_custom_agent_fields(state);
            }
        }
        CUSTOM_AGENT_TYPE_LABEL => {
            if state.custom_agents.selected_agent().is_some() {
                persist_custom_agents_update(state, |custom_agents| {
                    if let Some(agent) = custom_agents.agents.get_mut(custom_agents.selected_agent)
                    {
                        agent.agent.agent_type = next_custom_agent_type(agent.agent.agent_type);
                    }
                });
            }
        }
        CUSTOM_AGENT_ADD_LABEL => {
            persist_custom_agents_update(state, |custom_agents| {
                custom_agents
                    .agents
                    .push(default_custom_agent(&custom_agents.agents));
                custom_agents.selected_agent = custom_agents.agents.len().saturating_sub(1);
            });
        }
        CUSTOM_AGENT_DELETE_LABEL => {
            if state.custom_agents.selected_agent().is_some() {
                persist_custom_agents_update(state, |custom_agents| {
                    custom_agents.agents.remove(custom_agents.selected_agent);
                    if custom_agents.selected_agent >= custom_agents.agents.len() {
                        custom_agents.selected_agent = custom_agents.agents.len().saturating_sub(1);
                    }
                });
            }
        }
        CUSTOM_AGENT_ID_LABEL | CUSTOM_AGENT_DISPLAY_NAME_LABEL | CUSTOM_AGENT_COMMAND_LABEL => {
            if let Some(agent) = state.custom_agents.selected_agent() {
                state.edit_buffer = match field.label.as_str() {
                    CUSTOM_AGENT_ID_LABEL => agent.agent.id.clone(),
                    CUSTOM_AGENT_DISPLAY_NAME_LABEL => agent.agent.display_name.clone(),
                    CUSTOM_AGENT_COMMAND_LABEL => agent.agent.command.clone(),
                    _ => String::new(),
                };
                state.editing = true;
            }
        }
        _ => {}
    }
}

fn finish_custom_agent_edit(state: &mut SettingsState) {
    let Some(field) = state.fields.get(state.selected) else {
        state.editing = false;
        state.edit_buffer.clear();
        return;
    };

    let new_value = state.edit_buffer.clone();
    let label = field.label.clone();
    state.editing = false;
    state.edit_buffer.clear();

    let Some(_) = state.custom_agents.selected_agent() else {
        return;
    };

    persist_custom_agents_update(state, move |custom_agents| {
        if let Some(agent) = custom_agents.agents.get_mut(custom_agents.selected_agent) {
            match label.as_str() {
                CUSTOM_AGENT_ID_LABEL => agent.agent.id = new_value.clone(),
                CUSTOM_AGENT_DISPLAY_NAME_LABEL => agent.agent.display_name = new_value.clone(),
                CUSTOM_AGENT_COMMAND_LABEL => agent.agent.command = new_value.clone(),
                _ => {}
            }
        }
    });
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
            if state.category == SettingsCategory::CustomAgents {
                start_custom_agent_interaction(state);
                return;
            }
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
            if state.category == SettingsCategory::CustomAgents {
                finish_custom_agent_edit(state);
                return;
            }
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

/// Render the settings screen (borderless — outer pane border is handled by app.rs).
pub fn render(state: &SettingsState, frame: &mut Frame, area: Rect) {
    // Category sub-tab header line
    let active_idx = SettingsCategory::ALL
        .iter()
        .position(|c| *c == state.category)
        .unwrap_or(0);
    let labels: Vec<&str> = SettingsCategory::ALL.iter().map(|c| c.label()).collect();
    let tab_title = super::build_tab_title(&labels, active_idx);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new(tab_title);
    frame.render_widget(header, chunks[0]);
    render_fields(state, frame, chunks[1]);
}

/// Render the fields list for the current category.
fn render_fields(state: &SettingsState, frame: &mut Frame, area: Rect) {
    if state.fields.is_empty() {
        let block = Block::default();
        let paragraph = Paragraph::new("No settings in this category")
            .block(block)
            .style(theme::style::muted_text());
        frame.render_widget(paragraph, area);
        return;
    }

    let show_error = matches!(
        state.category,
        SettingsCategory::Voice | SettingsCategory::CustomAgents
    ) && state.save_error.is_some();

    let chunks = if show_error {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0)])
            .split(area)
    };

    let items: Vec<ListItem> = state
        .fields
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            let is_selected = idx == state.selected;
            let is_editing = is_selected && state.editing;

            let label_style = if is_selected {
                Style::default()
                    .fg(theme::color::TEXT_PRIMARY)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::TEXT_PRIMARY)
            };

            let value_display = if is_editing {
                format!("{}_", state.edit_buffer)
            } else {
                field.value.clone()
            };

            let value_style = match (&field.field_type, is_editing) {
                (_, true) => Style::default().fg(theme::color::ACTIVE),
                (FieldType::Bool, false) => {
                    if field.value == "true" {
                        Style::default().fg(theme::color::SUCCESS)
                    } else {
                        Style::default().fg(theme::color::ERROR)
                    }
                }
                (FieldType::Choice, false) => Style::default().fg(theme::color::FOCUS),
                (FieldType::Action, false) => Style::default().fg(theme::color::ACTIVE),
                (FieldType::Path, false) => Style::default().fg(theme::color::FOCUS),
                (FieldType::Text, false) => Style::default().fg(theme::color::TEXT_PRIMARY),
            };

            let type_indicator = match field.field_type {
                FieldType::Text => "T",
                FieldType::Bool => "B",
                FieldType::Path => "P",
                FieldType::Choice => "C",
                FieldType::Action => "A",
            };

            let bg_style = if is_selected {
                Style::default().bg(theme::color::SURFACE)
            } else {
                Style::default()
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("[{}] ", type_indicator),
                    theme::style::muted_text(),
                ),
                Span::styled(format!("{}: ", field.label), label_style),
                Span::styled(value_display, value_style),
            ]);
            ListItem::new(line).style(bg_style)
        })
        .collect();

    let hints = if state.editing {
        " Enter: save | Esc: cancel"
    } else if state.category == SettingsCategory::CustomAgents {
        " Enter: cycle/edit/action | Ctrl+Left/Right: category"
    } else {
        " Enter: edit | Space: toggle bool | Tab/Shift+Tab: category"
    };

    let block = Block::default().title(format!("{}{}", state.category.label(), hints));
    let list = List::new(items).block(block).highlight_style(theme::style::active_item());
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(state.selected));

    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    if let Some(error) = state.save_error.as_ref() {
        let error_block = Block::default().title("Save failed");
        let error_paragraph = Paragraph::new(error.as_str())
            .block(error_block)
            .style(Style::default().fg(theme::color::ERROR));

        let error_area = if chunks.len() > 1 { chunks[1] } else { area };
        frame.render_widget(error_paragraph, error_area);
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::custom_agents::load_custom_agents_from_path;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::Path;

    fn state_with_fields() -> SettingsState {
        let mut state = SettingsState::default();
        state.load_category_fields();
        state
    }

    fn voice_state_with_fields() -> SettingsState {
        let mut state = SettingsState::default();
        state.category = SettingsCategory::Voice;
        state.load_category_fields();
        state
    }

    fn custom_agents_state_with_fields(config_path: &Path) -> SettingsState {
        let mut state = SettingsState::default();
        state.category = SettingsCategory::CustomAgents;
        state.config_path_override = Some(config_path.to_path_buf());
        state.load_category_fields();
        state
    }

    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf[(x, y)].symbol());
            }
            text.push_str(line.trim_end());
            text.push('\n');
        }
        text
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
            assert!(!fields.is_empty(), "Category {:?} has no fields", cat);
        }
    }

    #[test]
    fn voice_category_has_expected_fields() {
        let fields = fields_for_category(SettingsCategory::Voice);
        let labels: Vec<_> = fields.iter().map(|field| field.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Model path",
                "Hotkey",
                "Input device",
                "Language",
                "Enabled",
            ]
        );
    }

    #[test]
    fn voice_fields_reflect_persisted_settings() {
        let voice = VoiceConfig {
            model_path: Some(std::path::PathBuf::from("/tmp/models/qwen")),
            hotkey: "Ctrl+Alt+V".to_string(),
            input_device: "mic-2".to_string(),
            language: "ja".to_string(),
            enabled: true,
        };
        let fields = voice_fields(Some(&voice));
        assert_eq!(fields.len(), 5);
        assert_eq!(fields[0].value, "/tmp/models/qwen");
        assert_eq!(fields[1].value, "Ctrl+Alt+V");
        assert_eq!(fields[2].value, "mic-2");
        assert_eq!(fields[3].value, "ja");
        assert_eq!(fields[4].value, "true");
    }

    #[test]
    fn save_settings_to_path_persists_voice_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let model_dir = dir.path().join("model");
        std::fs::create_dir(&model_dir).unwrap();

        let mut state = voice_state_with_fields();
        state.fields[0].value = model_dir.display().to_string();
        state.fields[1].value = "Ctrl+Shift+V".to_string();
        state.fields[2].value = "mic-9".to_string();
        state.fields[3].value = "en".to_string();
        state.fields[4].value = "true".to_string();

        save_settings_to_path(&state, &config_path).unwrap();

        let loaded = Settings::load_from_path(&config_path).unwrap();
        assert_eq!(
            loaded.voice.model_path.as_deref(),
            Some(model_dir.as_path())
        );
        assert_eq!(loaded.voice.hotkey, "Ctrl+Shift+V");
        assert_eq!(loaded.voice.input_device, "mic-9");
        assert_eq!(loaded.voice.language, "en");
        assert!(loaded.voice.enabled);
    }

    #[test]
    fn category_cycle_full_round() {
        let mut cat = SettingsCategory::General;
        for _ in 0..8 {
            cat = cat.next();
        }
        assert_eq!(cat, SettingsCategory::General); // full cycle
    }

    #[test]
    fn voice_and_skills_categories_are_last_in_sidebar_order() {
        assert_eq!(SettingsCategory::ALL.len(), 8);
        assert_eq!(SettingsCategory::ALL[6], SettingsCategory::Skills);
        assert_eq!(SettingsCategory::ALL[7], SettingsCategory::Voice);
    }

    #[test]
    fn category_prev_full_round() {
        let mut cat = SettingsCategory::General;
        for _ in 0..8 {
            cat = cat.prev();
        }
        assert_eq!(cat, SettingsCategory::General); // full cycle
    }

    #[test]
    fn skills_category_renders_bundled_count() {
        let mut state = SettingsState::default();
        state.category = SettingsCategory::Skills;
        state.load_category_fields();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let text = buffer_text(&buf);
        assert!(text.contains("Bundled skills"));
        assert!(text.contains("Skills"));
    }

    #[test]
    fn skills_bundled_count_is_positive() {
        let mut state = SettingsState::default();
        state.category = SettingsCategory::Skills;
        state.load_category_fields();
        assert_eq!(state.fields.len(), 1);
        assert_eq!(state.fields[0].label, "Bundled skills");
        let count: usize = state.fields[0].value.parse().unwrap_or(0);
        assert!(count > 0, "should have bundled skills");
    }

    #[test]
    fn custom_agents_category_loads_persisted_agent_fields() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[tools.customCodingAgents.my-agent]
id = "my-agent"
displayName = "My Agent"
agentType = "command"
command = "my-agent-cli"
"#,
        )
        .unwrap();

        let state = custom_agents_state_with_fields(&config_path);

        assert_eq!(state.fields.len(), 7);
        assert_eq!(state.fields[0].label, CUSTOM_AGENT_LABEL);
        assert_eq!(state.fields[1].value, "my-agent");
        assert_eq!(state.fields[2].value, "My Agent");
        assert_eq!(state.fields[3].value, "command");
        assert_eq!(state.fields[4].value, "my-agent-cli");
    }

    #[test]
    fn custom_agents_add_edit_delete_persist_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "debug = true\n").unwrap();

        let mut state = custom_agents_state_with_fields(&config_path);

        state.selected = 5;
        update(&mut state, SettingsMessage::StartEdit);

        let mut agents = load_custom_agents_from_path(&config_path).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].display_name, "Custom Agent 1");
        assert_eq!(agents[0].agent_type, CustomAgentType::Command);

        state.selected = 2;
        update(&mut state, SettingsMessage::StartEdit);
        state.edit_buffer = "QA Agent".to_string();
        update(&mut state, SettingsMessage::EndEdit);

        state.selected = 3;
        update(&mut state, SettingsMessage::StartEdit);

        state.selected = 4;
        update(&mut state, SettingsMessage::StartEdit);
        state.edit_buffer = "qa-agent-cli".to_string();
        update(&mut state, SettingsMessage::EndEdit);

        agents = load_custom_agents_from_path(&config_path).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].display_name, "QA Agent");
        assert_eq!(agents[0].agent_type, CustomAgentType::Path);
        assert_eq!(agents[0].command, "qa-agent-cli");

        state.selected = 6;
        update(&mut state, SettingsMessage::StartEdit);

        agents = load_custom_agents_from_path(&config_path).unwrap();
        assert!(agents.is_empty());
        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("debug = true"));
        assert!(!content.contains("customCodingAgents"));
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
        assert!(text.contains("General"));
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
    fn render_voice_save_error_is_visible() {
        let mut state = voice_state_with_fields();
        state.fields[0].value = "/nonexistent/model".to_string();
        update(&mut state, SettingsMessage::Save);
        assert!(state.save_error.is_some());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(&state, f, area);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let text = buffer_text(&buf);
        assert!(text.contains("Save failed"));
        assert!(text.contains("voice model path does not exist"));
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
